use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

fn release(version: &str) -> Release {
    Release {
        commit: "0".repeat(40),
        version: Version::parse(version).unwrap(),
        tag: format!("v{version}"),
        notes: String::new(),
    }
}

/// A gzipped package tarball as `npm pack` lays it out.
fn tarball(package_json: &Value) -> Vec<u8> {
    let manifest = serde_json::to_vec(package_json).unwrap();
    let mut builder = tar::Builder::new(Vec::new());
    for (path, contents) in [
        ("package/package.json", manifest.as_slice()),
        ("package/index.js", b"module.exports = 1;\n".as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder.append_data(&mut header, path, contents).unwrap();
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&builder.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}

fn package_json(name: &str, version: &str) -> Value {
    json!({ "name": name, "version": version, "description": "A Pi extension", "keywords": ["pi-package"] })
}

/// A tarball written to a temporary directory, which the publisher reads.
struct Tarball {
    _directory: tempfile::TempDir,
    path: String,
    bytes: Vec<u8>,
}

fn write_tarball(name: &str, version: &str) -> Tarball {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("package.tgz");
    let bytes = tarball(&package_json(name, version));
    std::fs::write(&path, &bytes).unwrap();
    Tarball {
        path: path.to_str().unwrap().to_owned(),
        _directory: directory,
        bytes,
    }
}

fn integrity(bytes: &[u8]) -> String {
    format!("sha512-{}", shared::base64(Sha512::digest(bytes)))
}

fn declaration(package: &str) -> NpmDeclaration {
    NpmDeclaration {
        name: "@scope/pi-extension".into(),
        package: package.into(),
        registry: "https://registry.example.org".into(),
        access: Access::Public,
        tag: "presswerk".into(),
        aliases: vec!["latest".into()],
        token: Some(Credential::env("NPM_TOKEN")),
        username: None,
        password: None,
    }
}

fn decode(declaration: &Value) -> Result<Npm, DeclarationError> {
    Npm::from_declaration(declaration)
}

#[derive(Debug, Default)]
struct FakeState {
    packument: Option<Packument>,
    published: Vec<Value>,
    dist_tags_set: Vec<(String, String)>,
    /// Every upload fails; with `lands`, after the registry stored it anyway.
    publish_fails: bool,
    lands: bool,
    /// The registry accepts uploads but never shows them.
    hides_uploads: bool,
    no_dist_tag_api: bool,
    /// The dist-tags as the dist-tag API reports them, when the package document lags behind.
    live_dist_tags: Option<BTreeMap<String, String>>,
    dist_tags_unreadable: bool,
}

#[derive(Debug, Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);

impl Fake {
    fn with_version(self, version: &str, integrity: Option<String>) -> Self {
        self.state()
            .packument
            .get_or_insert_default()
            .versions
            .insert(version.into(), integrity);
        self
    }

    fn with_dist_tag(self, tag: &str, version: &str) -> Self {
        self.state()
            .packument
            .get_or_insert_default()
            .dist_tags
            .insert(tag.into(), version.into());
        self
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }

    fn dist_tags(&self) -> BTreeMap<String, String> {
        self.state().packument.clone().unwrap_or_default().dist_tags
    }

    fn store(state: &mut FakeState, document: &Value) {
        let packument = state.packument.get_or_insert_default();
        for (version, manifest) in document["versions"].as_object().unwrap() {
            packument.versions.insert(
                version.clone(),
                manifest["dist"]["integrity"].as_str().map(str::to_owned),
            );
        }
        for (tag, version) in document["dist-tags"].as_object().unwrap() {
            packument
                .dist_tags
                .insert(tag.clone(), version.as_str().unwrap().into());
        }
    }
}

impl NpmRegistry for Fake {
    fn packument(&self, _name: &str) -> Result<Option<Packument>, PublicationError> {
        Ok(self.state().packument.clone())
    }

    fn publish(&self, _name: &str, document: &Value) -> Result<(), PublicationError> {
        let mut state = self.state();
        state.published.push(document.clone());
        if state.publish_fails {
            if state.lands {
                Self::store(&mut state, document);
            }
            return Err(PublicationError::new(
                "HTTP 403: cannot publish over the previously published versions",
            ));
        }
        if !state.hides_uploads {
            Self::store(&mut state, document);
        }
        Ok(())
    }

    fn dist_tags(&self, _name: &str) -> Result<BTreeMap<String, String>, DistTagError> {
        let state = self.state();
        if state.no_dist_tag_api {
            return Err(DistTagError::Unsupported("HTTP 405".into()));
        }
        if state.dist_tags_unreadable {
            return Err(DistTagError::Failed("HTTP 503".into()));
        }
        Ok(state
            .live_dist_tags
            .clone()
            .unwrap_or_else(|| state.packument.clone().unwrap_or_default().dist_tags))
    }

    fn set_dist_tag(&self, _name: &str, tag: &str, version: &str) -> Result<(), DistTagError> {
        let mut state = self.state();
        if state.no_dist_tag_api {
            return Err(DistTagError::Unsupported("HTTP 405".into()));
        }
        state.dist_tags_set.push((tag.into(), version.into()));
        if let Some(live) = &mut state.live_dist_tags {
            live.insert(tag.into(), version.into());
        }
        state
            .packument
            .get_or_insert_default()
            .dist_tags
            .insert(tag.into(), version.into());
        Ok(())
    }
}

fn publisher(fake: &Fake, declaration: NpmDeclaration) -> NpmPublisher {
    NpmPublisher::new(Box::new(fake.clone()), declaration).with_verify_delays(Vec::new())
}

fn steps(steps: &ArtifactSteps) -> Vec<(String, String, AssetStep)> {
    steps
        .iter()
        .map(|step| {
            (
                step.kind.label.to_string(),
                step.name.clone(),
                step.step.clone(),
            )
        })
        .collect()
}

#[test]
fn decodes_a_declaration_with_a_token_or_a_username_and_password() {
    let valid = json!({
        "name": "@scope/pi-extension",
        "package": "/nix/store/00000000000000000000000000000000-pi-extension.tgz",
        "registry": "https://registry.npmjs.org",
        "access": "public",
        "tag": "presswerk",
        "aliases": ["latest", "release-{major}.{minor}"],
        "token": { "env": "NPM_TOKEN" }
    });
    let npm = decode(&valid).unwrap();
    assert_eq!(
        npm.credentials(),
        [("token", &Credential::env("NPM_TOKEN"))]
    );
    assert_eq!(
        npm.artifacts(),
        [(
            "package".to_owned(),
            "/nix/store/00000000000000000000000000000000-pi-extension.tgz".to_owned()
        )]
    );

    let with = |changes: Value| {
        let mut declaration = valid.clone();
        for (key, value) in changes.as_object().unwrap() {
            if value.is_null() {
                declaration.as_object_mut().unwrap().remove(key);
            } else {
                declaration[key] = value.clone();
            }
        }
        decode(&declaration)
    };
    let basic = with(json!({
        "token": null,
        "username": { "value": "_json_key_base64" },
        "password": { "env": "REGISTRY_PASSWORD" }
    }))
    .unwrap();
    assert_eq!(
        basic.credentials(),
        [
            ("username", &Credential::Value("_json_key_base64".into())),
            ("password", &Credential::env("REGISTRY_PASSWORD")),
        ]
    );

    for (changes, expected) in [
        (json!({ "registry": "http://localhost:4873/" }), None),
        (json!({ "name": "pi-extension" }), None),
        (json!({ "access": "restricted", "aliases": [] }), None),
        (
            json!({ "token": null }),
            Some("either token, or username and password"),
        ),
        (
            json!({ "username": { "value": "user" }, "password": { "env": "PASSWORD" } }),
            Some("either token, or username and password"),
        ),
        (
            json!({ "token": null, "username": { "value": "user" } }),
            Some("either token, or username and password"),
        ),
        (
            json!({ "token": { "value": "npm_secret" } }),
            Some("must be a secret"),
        ),
        (
            json!({ "token": null, "username": { "value": "user" }, "password": { "value": "secret" } }),
            Some("must be a secret"),
        ),
        (
            json!({ "registry": "https://user:secret@registry.npmjs.org" }),
            Some("without credentials"),
        ),
        (
            json!({ "registry": "ftp://registry.npmjs.org" }),
            Some("HTTP(S) base URL"),
        ),
        (
            json!({ "package": "pi-extension.tgz" }),
            Some("absolute store path"),
        ),
        (json!({ "name": "scope/pi-extension" }), Some("@scope/name")),
        (json!({ "name": "@scope" }), Some("@scope/name")),
        (json!({ "tag": "v1" }), Some(DIST_TAG_RULE)),
        (json!({ "tag": "||" }), Some(DIST_TAG_RULE)),
        (json!({ "tag": "2026-lts" }), None),
        (
            json!({ "registry": "https://registry.example.org:8443/npm/" }),
            None,
        ),
        // Claims name the registry as written, so it is written the one way the client reads it.
        (
            json!({ "registry": "https://Registry.npmjs.org" }),
            Some("written canonically"),
        ),
        (
            json!({ "registry": "https://registry.npmjs.org:443/" }),
            Some("written canonically"),
        ),
        (
            json!({ "registry": "https://registry.npmjs.org//npm" }),
            Some("written canonically"),
        ),
        (
            json!({ "registry": "https://registry.npmjs.org/a/../npm" }),
            Some("written canonically"),
        ),
        (
            json!({ "aliases": ["{version}"] }),
            Some("must not contain {version}"),
        ),
        (
            json!({ "aliases": ["{major}"] }),
            Some("renders to 1 for version 1.2.3"),
        ),
        (json!({ "aliases": ["presswerk"] }), Some("is the tag")),
        (
            json!({ "aliases": ["latest", "latest"] }),
            Some("declared twice"),
        ),
        (json!({ "access": "private" }), Some("unknown variant")),
        (json!({ "unknown": true }), Some("unknown field")),
    ] {
        match (with(changes.clone()), expected) {
            (Ok(_), None) => {}
            (Err(error), Some(expected)) => {
                assert!(error.to_string().contains(expected), "{changes}: {error}");
            }
            (result, _) => panic!("{changes}: {:?}", result.map(|_| ())),
        }
    }
}

#[test]
fn rejects_dist_tags_that_npm_reads_as_a_version_or_range() {
    // Checked against npm's `semver.validRange`.
    for tag in [
        "1",
        "1.2",
        "1.2.3",
        "v1",
        "=1",
        "=v1",
        "vv1",
        "==1",
        "^1",
        "~1.2",
        "~>1",
        ">=1",
        "<2",
        "x",
        "X",
        "*",
        "~x.1",
        "1.x",
        " 1",
        "",
        " ",
        "\t1",
        "||",
        "1 || 2",
        "1||2",
        "1 - 2",
        "1 -  2",
        ">= 1",
        "~ 1.2",
        "^ 1",
        "1 2",
        "1.2.3-rc.1",
        "1.2.3-0a",
        "1.2.3+build.5",
        "1.2.x-rc",
        ">=1.2.3-rc.1 <2",
    ] {
        assert!(looks_like_version(tag), "{tag:?}");
    }
    // A few odd forms that npm would accept are rejected too; the rule errs on the safe side.
    for tag in ["x.1", "1.x.1", "> = 1"] {
        assert!(looks_like_version(tag), "{tag:?}");
    }
    for tag in [
        "latest",
        "next",
        "presswerk",
        "vnext",
        "v",
        "xyz",
        "release-1.2",
        "beta1",
        "stable",
        "2026-lts",
        "v2-next",
        "1abc",
        "01",
        "1.2-rc",
        "1.2.3-rc.01",
        "V1",
        "v 1",
        "a||b",
        "x || latest",
        "1 -2",
        ">",
        "~",
        "=",
        "<1 >",
    ] {
        assert!(!looks_like_version(tag), "{tag:?}");
    }
}

#[test]
fn describes_the_version_the_tag_and_the_aliases() {
    let mut declaration = declaration("/nix/store/package.tgz");
    declaration.aliases = vec!["latest".into(), "release-{major}.{minor}".into()];
    assert_eq!(
        describe(&declaration, &Version::new(1, 4, 0)).unwrap(),
        Description {
            target: "https://registry.example.org/@scope/pi-extension".into(),
            items: vec![
                ("version".into(), "@scope/pi-extension@1.4.0".into()),
                ("tag".into(), "presswerk".into()),
                ("alias".into(), "latest".into()),
                ("alias".into(), "release-1.4".into()),
            ],
        }
    );
}

#[test]
fn reads_the_manifest_and_digests_of_a_tarball() {
    let bytes = tarball(&package_json("@scope/pi-extension", "1.2.3"));
    let local = LocalPackage::from_bytes(bytes.clone()).unwrap();
    assert_eq!(local.name(), "@scope/pi-extension");
    assert_eq!(local.version(), "1.2.3");
    assert_eq!(local.integrity(), integrity(&bytes));

    let document =
        local.publish_document("https://registry.example.org/", "presswerk", Access::Public);
    assert_eq!(document["_id"], "@scope/pi-extension");
    assert_eq!(document["description"], "A Pi extension");
    assert_eq!(document["access"], "public");
    assert_eq!(document["dist-tags"], json!({ "presswerk": "1.2.3" }));
    let manifest = &document["versions"]["1.2.3"];
    assert_eq!(manifest["_id"], "@scope/pi-extension@1.2.3");
    assert_eq!(manifest["keywords"], json!(["pi-package"]));
    assert_eq!(manifest["dist"]["integrity"], integrity(&bytes));
    assert_eq!(
        manifest["dist"]["shasum"],
        Sha1::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        manifest["dist"]["tarball"],
        "https://registry.example.org/@scope/pi-extension/-/@scope/pi-extension-1.2.3.tgz"
    );
    let attachment = &document["_attachments"]["@scope/pi-extension-1.2.3.tgz"];
    assert_eq!(attachment["data"], shared::base64(&bytes));
    assert_eq!(attachment["length"], bytes.len());

    for (contents, expected) in [
        (b"not gzip".to_vec(), "failed"),
        (tarball(&json!({ "name": "pi-extension" })), "no version"),
        (tarball(&json!(["no object"])), "no JSON object"),
    ] {
        let error = LocalPackage::from_bytes(contents).unwrap_err();
        assert!(error.contains(expected) || expected == "failed", "{error}");
    }
}

#[test]
fn finds_the_one_tarball_in_a_directory() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = tarball(&package_json("pi-extension", "1.2.3"));
    let error = LocalPackage::read(directory.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("exactly one .tgz, but holds 0"), "{error}");
    std::fs::write(directory.path().join("pi-extension-1.2.3.tgz"), &bytes).unwrap();
    std::fs::write(directory.path().join("README"), "not a tarball").unwrap();
    assert_eq!(
        LocalPackage::read(directory.path()).unwrap().integrity(),
        integrity(&bytes)
    );
    std::fs::write(directory.path().join("other.tgz"), &bytes).unwrap();
    let error = LocalPackage::read(directory.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("but holds 2"), "{error}");
}

#[test]
fn publishes_a_new_version_with_its_tag_and_moves_the_aliases() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default()
        .with_version("1.1.0", Some(integrity(b"older")))
        .with_dist_tag("latest", "1.1.0");
    let publisher = publisher(&fake, declaration(&tarball.path));
    let release = release("1.2.0");
    let ctx = Context::detached();

    assert_eq!(
        publisher.state(&release, &ctx).unwrap(),
        PublisherState::Missing
    );
    let reported = Mutex::new(Vec::new());
    let progress = |step: &ArtifactStep| reported.lock().unwrap().push(step.clone());
    let published = publisher
        .publish(&release, &Context::detached().with_progress(&progress))
        .unwrap();
    assert_eq!(
        steps(&published),
        [
            (
                "package".into(),
                "@scope/pi-extension@1.2.0".into(),
                AssetStep::Uploaded
            ),
            (
                "alias".into(),
                "@scope/pi-extension@latest".into(),
                AssetStep::Uploaded
            ),
        ]
    );
    assert_eq!(*reported.lock().unwrap(), published);
    let document = fake.state().published[0].clone();
    assert_eq!(document["dist-tags"], json!({ "presswerk": "1.2.0" }));
    assert_eq!(document["access"], "public");
    assert_eq!(
        document["versions"]["1.2.0"]["dist"]["integrity"],
        integrity(&tarball.bytes)
    );
    assert_eq!(
        fake.dist_tags(),
        BTreeMap::from([
            ("latest".into(), "1.2.0".into()),
            ("presswerk".into(), "1.2.0".into()),
        ])
    );
    assert_eq!(
        publisher.state(&release, &ctx).unwrap(),
        PublisherState::Published
    );
    assert_eq!(
        publisher.completeness(&release, &ctx).unwrap(),
        Completeness::Complete
    );
}

#[test]
fn a_rerun_keeps_the_version_and_never_touches_the_tag() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    // The tag has moved on to a later release since, which is no conflict.
    let fake = Fake::default()
        .with_version(
            "1.2.0",
            Some(format!("{} sha1-ignored", integrity(&tarball.bytes))),
        )
        .with_dist_tag("presswerk", "1.3.0")
        .with_dist_tag("latest", "1.3.0");
    let publisher = publisher(&fake, declaration(&tarball.path));
    let release = release("1.2.0");
    let ctx = Context::detached();

    assert_eq!(
        publisher.state(&release, &ctx).unwrap(),
        PublisherState::Published
    );
    let expected = [
        (
            "package".into(),
            "@scope/pi-extension@1.2.0".into(),
            AssetStep::AlreadyUploaded,
        ),
        (
            "alias".into(),
            "@scope/pi-extension@latest".into(),
            AssetStep::Kept("1.3.0 is higher".into()),
        ),
    ];
    assert_eq!(
        steps(&publisher.complete_steps(&release, &ctx).unwrap()),
        expected
    );
    assert_eq!(steps(&publisher.publish(&release, &ctx).unwrap()), expected);
    assert!(fake.state().published.is_empty());
    assert!(fake.state().dist_tags_set.is_empty());
}

#[test]
fn a_complete_version_with_a_lower_alias_moves_only_the_alias() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default()
        .with_version("1.2.0", Some(integrity(&tarball.bytes)))
        .with_dist_tag("latest", "1.1.9");
    let publisher = publisher(&fake, declaration(&tarball.path));
    let release = release("1.2.0");

    assert_eq!(
        publisher.state(&release, &Context::detached()).unwrap(),
        PublisherState::AliasesPending
    );
    let published = publisher.publish(&release, &Context::detached()).unwrap();
    assert_eq!(published[0].step, AssetStep::AlreadyUploaded);
    assert_eq!(published[1].step, AssetStep::Uploaded);
    assert!(fake.state().published.is_empty());
    assert_eq!(
        fake.state().dist_tags_set,
        [("latest".into(), "1.2.0".into())]
    );
}

#[test]
fn another_tarball_under_the_version_is_conflicting() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default().with_version("1.2.0", Some(integrity(b"other bytes")));
    let publisher = publisher(&fake, declaration(&tarball.path));
    let release = release("1.2.0");

    let PublisherState::Conflicting(reason) =
        publisher.state(&release, &Context::detached()).unwrap()
    else {
        panic!("not conflicting");
    };
    assert!(
        reason.contains("exists in https://registry.example.org with another tarball"),
        "{reason}"
    );
    let error = publisher
        .publish(&release, &Context::detached())
        .unwrap_err()
        .to_string();
    assert!(error.contains("another tarball"), "{error}");
    assert!(fake.state().published.is_empty());
    assert!(fake.state().dist_tags_set.is_empty());
}

#[test]
fn a_version_without_a_sha512_integrity_is_unknown() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    for integrity in [None, Some("sha1-AAAA".to_owned())] {
        let fake = Fake::default().with_version("1.2.0", integrity);
        let publisher = publisher(&fake, declaration(&tarball.path));
        let error = publisher
            .state(&release("1.2.0"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert!(error.contains("no sha512 integrity"), "{error}");
        assert!(
            publisher
                .publish(&release("1.2.0"), &Context::detached())
                .is_err()
        );
        assert!(fake.state().published.is_empty());
    }
}

#[test]
fn a_tarball_of_another_package_or_version_is_never_uploaded() {
    for (name, version, expected) in [
        ("other-package", "1.2.0", "is named \"other-package\""),
        ("@scope/pi-extension", "1.2.1", "has version \"1.2.1\""),
    ] {
        let tarball = write_tarball(name, version);
        let fake = Fake::default();
        let publisher = publisher(&fake, declaration(&tarball.path));
        let error = publisher
            .publish(&release("1.2.0"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        assert!(fake.state().published.is_empty());
    }
}

#[test]
fn an_alias_moves_only_to_a_higher_final_version() {
    let tarball = write_tarball("@scope/pi-extension", "1.4.1");
    let fake = Fake::default()
        .with_dist_tag("latest", "2.0.0")
        .with_dist_tag("release-1.4", "1.4.0")
        .with_dist_tag("odd", "not-a-version");
    let mut declaration = declaration(&tarball.path);
    declaration.aliases = vec![
        "latest".into(),
        "release-{major}.{minor}".into(),
        "odd".into(),
    ];
    let publisher = publisher(&fake, declaration);

    let published = publisher
        .publish(&release("1.4.1"), &Context::detached())
        .unwrap();
    assert_eq!(
        steps(&published)[1..],
        [
            (
                "alias".into(),
                "@scope/pi-extension@latest".into(),
                AssetStep::Kept("2.0.0 is higher".into())
            ),
            (
                "alias".into(),
                "@scope/pi-extension@release-1.4".into(),
                AssetStep::Uploaded
            ),
            (
                "alias".into(),
                "@scope/pi-extension@odd".into(),
                AssetStep::Kept("points to \"not-a-version\", no version".into())
            ),
        ]
    );
    assert_eq!(
        fake.state().dist_tags_set,
        [("release-1.4".into(), "1.4.1".into())]
    );
}

#[test]
fn a_registry_without_the_dist_tag_api_only_warns() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default();
    fake.state().no_dist_tag_api = true;
    let publisher = publisher(&fake, declaration(&tarball.path));

    let published = publisher
        .publish(&release("1.2.0"), &Context::detached())
        .unwrap();
    assert_eq!(published[0].step, AssetStep::Uploaded);
    let AssetStep::Warning(warning) = &published[1].step else {
        panic!("{:?}", published[1]);
    };
    assert!(
        warning.starts_with("https://registry.example.org has no dist-tag API (HTTP 405)"),
        "{warning}"
    );
}

#[test]
fn a_failed_upload_is_inspected_again() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");

    // A concurrent run published the same tarball.
    let fake = Fake::default();
    fake.state().publish_fails = true;
    fake.state().lands = true;
    let publisher = publisher(&fake, declaration(&tarball.path));
    let published = publisher
        .publish(&release("1.2.0"), &Context::detached())
        .unwrap();
    assert_eq!(published[0].step, AssetStep::Uploaded);

    // Nothing landed: the upload's own error is reported.
    let fake = Fake::default();
    fake.state().publish_fails = true;
    let publisher = self::publisher(&fake, declaration(&tarball.path));
    let error = publisher
        .publish(&release("1.2.0"), &Context::detached())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot publish over"), "{error}");
    assert!(fake.state().dist_tags_set.is_empty());
}

#[test]
fn an_upload_the_registry_never_shows_fails_before_the_aliases() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default();
    fake.state().hides_uploads = true;
    let publisher = NpmPublisher::new(Box::new(fake.clone()), declaration(&tarball.path))
        .with_verify_delays(vec![Duration::ZERO; 2]);
    let error = publisher
        .publish(&release("1.2.0"), &Context::detached())
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "@scope/pi-extension@1.2.0 was not published: Missing"
    );
    assert_eq!(fake.state().published.len(), 1);
    assert!(fake.state().dist_tags_set.is_empty());
}

#[test]
fn completeness_needs_only_the_version_in_the_registry() {
    let declaration = declaration("/nix/store/package.tgz");
    let release = release("1.2.0");
    assert_eq!(
        completeness_with(
            &Fake::default().with_version("1.2.0", None),
            &declaration,
            &release
        ),
        Completeness::Complete
    );
    for (fake, reason) in [
        (Fake::default(), "the package does not exist"),
        (
            Fake::default().with_version("1.1.0", None),
            "the version is missing",
        ),
    ] {
        assert_eq!(
            completeness_with(&fake, &declaration, &release),
            Completeness::Incomplete {
                target: "npm package @scope/pi-extension@1.2.0 in https://registry.example.org"
                    .into(),
                reason: reason.into(),
            }
        );
    }
}

#[test]
fn an_alias_is_decided_by_the_live_dist_tags_not_a_cached_document() {
    let tarball = write_tarball("@scope/pi-extension", "1.4.0");
    // The package document still shows `latest` at 1.3.0, but 1.5.0 was released since.
    let fake = Fake::default()
        .with_version("1.4.0", Some(integrity(&tarball.bytes)))
        .with_dist_tag("latest", "1.3.0");
    fake.state().live_dist_tags = Some(BTreeMap::from([("latest".into(), "1.5.0".into())]));
    let publisher = publisher(&fake, declaration(&tarball.path));

    let published = publisher
        .publish(&release("1.4.0"), &Context::detached())
        .unwrap();
    assert_eq!(published[1].step, AssetStep::Kept("1.5.0 is higher".into()));
    assert!(fake.state().dist_tags_set.is_empty());
}

#[test]
fn an_alias_that_cannot_be_read_is_not_written() {
    let tarball = write_tarball("@scope/pi-extension", "1.2.0");
    let fake = Fake::default();
    fake.state().dist_tags_unreadable = true;
    let publisher = publisher(&fake, declaration(&tarball.path));

    let published = publisher
        .publish(&release("1.2.0"), &Context::detached())
        .unwrap();
    assert_eq!(published[0].step, AssetStep::Uploaded);
    assert_eq!(published[1].step, AssetStep::Warning("HTTP 503".into()));
    assert!(fake.state().dist_tags_set.is_empty());
}

/// One request the loopback registry received.
#[derive(Debug)]
struct Recorded {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: String,
}

/// A registry on a loopback port that answers each connection with the next recorded response.
fn serve(responses: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Recorded>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = BTreeMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let Some((name, value)) = line.trim_end().split_once(": ") else {
                    break;
                };
                headers.insert(name.to_ascii_lowercase(), value.to_owned());
            }
            let length = headers
                .get("content-length")
                .map_or(0, |length| length.parse().unwrap());
            let mut request_body = vec![0; length];
            reader.read_exact(&mut request_body).unwrap();
            sender
                .send(Recorded {
                    request_line: request_line.trim_end().to_owned(),
                    headers,
                    body: String::from_utf8(request_body).unwrap(),
                })
                .unwrap();
            write!(
                &stream,
                "HTTP/1.1 {status} Recorded\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (format!("http://{address}/npm/"), receiver)
}

#[test]
fn the_client_reads_package_documents() {
    let (registry, requests) = serve(vec![
        (
            200,
            r#"{"name":"@scope/pi-extension","dist-tags":{"latest":"1.1.0"},"versions":{"1.1.0":{"dist":{"integrity":"sha512-AAAA","shasum":"00"}},"1.0.0":{}}}"#,
        ),
        (404, r#"{"error":"Not found"}"#),
        (401, r#"{"error":"authentication required"}"#),
    ]);
    let client = RegistryClient::new(
        &registry,
        Some(Authentication::Token("npm-test-token".into())),
    )
    .unwrap();

    let packument = client.packument("@scope/pi-extension").unwrap().unwrap();
    assert_eq!(
        packument,
        Packument {
            dist_tags: BTreeMap::from([("latest".into(), "1.1.0".into())]),
            versions: BTreeMap::from([
                ("1.0.0".into(), None),
                ("1.1.0".into(), Some("sha512-AAAA".into())),
            ]),
        }
    );
    let request = requests.recv().unwrap();
    assert_eq!(
        request.request_line,
        "GET /npm/@scope%2Fpi-extension HTTP/1.1"
    );
    assert_eq!(request.headers["authorization"], "Bearer npm-test-token");
    assert!(request.headers["accept"].starts_with("application/vnd.npm.install-v1+json"));

    assert_eq!(client.packument("@scope/pi-extension").unwrap(), None);
    let error = client
        .packument("@scope/pi-extension")
        .unwrap_err()
        .to_string();
    assert!(
        error.ends_with("HTTP 401: authentication required"),
        "{error}"
    );
}

#[test]
fn the_client_publishes_and_sets_dist_tags() {
    let (registry, requests) = serve(vec![
        (200, r#"{"latest":"1.1.0","presswerk":"1.2.0"}"#),
        (405, r#"{"error":"Method not allowed"}"#),
        (200, r#"{"ok":true}"#),
        (
            403,
            r#"{"error":"You cannot publish over the previously published versions: 1.2.0."}"#,
        ),
        (201, r#"{"ok":true}"#),
        (405, r#"{"error":"Method not allowed"}"#),
        (500, "upstream failed"),
    ]);
    let client = RegistryClient::new(
        &registry,
        Some(Authentication::Basic {
            username: "_json_key_base64".into(),
            password: "npm-test-password".into(),
        }),
    )
    .unwrap();

    assert_eq!(
        client.dist_tags("@scope/pi-extension").unwrap(),
        BTreeMap::from([
            ("latest".into(), "1.1.0".into()),
            ("presswerk".into(), "1.2.0".into()),
        ])
    );
    let request = requests.recv().unwrap();
    assert_eq!(
        request.request_line,
        "GET /npm/-/package/@scope%2Fpi-extension/dist-tags HTTP/1.1"
    );
    assert_eq!(
        client.dist_tags("pi-extension"),
        Err(DistTagError::Unsupported(
            "HTTP 405: Method not allowed".into()
        ))
    );
    requests.recv().unwrap();

    let document = json!({ "name": "pi-extension" });
    client.publish("pi-extension", &document).unwrap();
    let request = requests.recv().unwrap();
    assert_eq!(request.request_line, "PUT /npm/pi-extension HTTP/1.1");
    assert_eq!(
        request.headers["authorization"],
        format!(
            "Basic {}",
            shared::base64("_json_key_base64:npm-test-password")
        )
    );
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).unwrap(),
        document
    );

    let error = client
        .publish("pi-extension", &document)
        .unwrap_err()
        .to_string();
    assert!(
        error.ends_with("refused to publish pi-extension: HTTP 403: You cannot publish over the previously published versions: 1.2.0."),
        "{error}"
    );
    requests.recv().unwrap();

    client
        .set_dist_tag("@scope/pi-extension", "latest", "1.2.0")
        .unwrap();
    let request = requests.recv().unwrap();
    assert_eq!(
        request.request_line,
        "PUT /npm/-/package/@scope%2Fpi-extension/dist-tags/latest HTTP/1.1"
    );
    assert_eq!(request.body, "\"1.2.0\"");

    assert_eq!(
        client.set_dist_tag("pi-extension", "latest", "1.2.0"),
        Err(DistTagError::Unsupported(
            "HTTP 405: Method not allowed".into()
        ))
    );
    assert_eq!(
        client.set_dist_tag("pi-extension", "latest", "1.2.0"),
        Err(DistTagError::Failed(
            "could not set dist-tag latest of pi-extension: HTTP 500: upstream failed".into()
        ))
    );
}

#[test]
fn credentials_never_appear_in_debug_output() {
    let client = RegistryClient::new(
        "https://registry.example.org",
        Some(Authentication::Token("npm-debug-sentinel".into())),
    )
    .unwrap();
    assert!(!format!("{client:?}").contains("npm-debug-sentinel"));
}

#[test]
fn errors_never_show_the_credentials_as_sent() {
    let encoded = shared::base64("_json_key_base64:npm-echo-sentinel");
    let echo: &'static str = Box::leak(
        format!(r#"{{"error":"denied for Basic {encoded} (npm-echo-sentinel)"}}"#).into_boxed_str(),
    );
    let (registry, _requests) = serve(vec![
        (403, echo),
        (401, r#"{"error":"bad token npm-token-sentinel"}"#),
    ]);
    let basic = RegistryClient::new(
        &registry,
        Some(Authentication::Basic {
            username: "_json_key_base64".into(),
            password: "npm-echo-sentinel".into(),
        }),
    )
    .unwrap();
    let error = basic
        .publish("pi-extension", &json!({}))
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains(&encoded) && !error.contains("npm-echo-sentinel"),
        "{error}"
    );
    assert!(error.contains("denied for [redacted]"), "{error}");

    let token = RegistryClient::new(
        &registry,
        Some(Authentication::Token("npm-token-sentinel".into())),
    )
    .unwrap();
    let error = token.packument("pi-extension").unwrap_err().to_string();
    assert!(!error.contains("npm-token-sentinel"), "{error}");
}

#[test]
fn a_stalled_registry_fails_the_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Accepts the connection, then never answers.
    let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
    let client = RegistryClient::new(&format!("http://{address}"), None)
        .unwrap()
        .with_timeouts(Duration::from_secs(5), Duration::from_millis(200));
    let started = std::time::Instant::now();
    assert!(client.packument("pi-extension").is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(held.join());
}
