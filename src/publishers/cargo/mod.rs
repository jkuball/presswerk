//! The Cargo publisher: uploads a packaged `.crate` to a registry with a sparse index, such as
//! crates.io or kellnr, through the publish API.

pub mod package;
pub mod registry;

use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::shared::secrets::Credential;
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, WarningLevel, shared,
};
use package::LocalCrate;
use registry::{CargoRegistry, HttpRegistry, IndexEntry};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use url::Url;

/// One version of a crate on a registry.
pub const CRATE: ArtifactKind = ArtifactKind::new("crate", "uploaded");

const LABELS: Labels = Labels {
    publisher: "registry",
    artifacts: "crates",
};

/// `publishers.cargo.<name>`: one crate uploaded to a Cargo registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoDeclaration {
    /// The crate's name on the index.
    pub name: String,
    /// The `.crate` archive, or a directory with exactly one, as a path in the Nix store.
    #[serde(rename = "crate")]
    pub archive: String,
    /// The sparse index, such as `sparse+https://index.crates.io/`.
    pub index: String,
    pub token: Credential,
}

impl CargoDeclaration {
    fn validate(&self) -> Result<(), DeclarationError> {
        let invalid = |message: String| Err(DeclarationError::Invalid(message));
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
        {
            return invalid(format!(
                "crate name {:?} must consist of ASCII letters, digits, `-`, and `_`",
                self.name
            ));
        }
        if !self.archive.starts_with('/') {
            return invalid(format!(
                "crate must be an absolute store path, not {:?}",
                self.archive
            ));
        }
        let index = self.index.strip_prefix("sparse+").map(Url::parse);
        let Some(Ok(index)) = index else {
            return invalid(format!(
                "Cargo index {:?} must be a sparse index URL, such as \"sparse+https://index.crates.io/\"",
                self.index
            ));
        };
        if !matches!(index.scheme(), "http" | "https")
            || index.cannot_be_a_base()
            || !index.username().is_empty()
            || index.password().is_some()
            || index.query().is_some()
            || index.fragment().is_some()
            || !self.index.ends_with('/')
        {
            return invalid(format!(
                "Cargo index {:?} must be an HTTP(S) URL without credentials, query, or fragment that ends in a slash",
                self.index
            ));
        }
        self.token.validate_secret("registry token")
    }
}

/// The `cargo` publisher type.
pub struct Cargo {
    declaration: CargoDeclaration,
    /// Built on first use, once the crate is fetched and the token is known.
    runner: OnceLock<CargoPublisher<HttpRegistry>>,
}

impl Cargo {
    #[must_use]
    pub fn declaration(&self) -> &CargoDeclaration {
        &self.declaration
    }

    fn runner(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<&CargoPublisher<HttpRegistry>, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let declaration = &self.declaration;
        let registry = HttpRegistry::new(
            &declaration.index,
            Some(ctx.session.secrets.get(&declaration.token)?),
        )?;
        let runner = CargoPublisher::new(
            registry,
            &declaration.index,
            &declaration.name,
            &declaration.archive,
            &release.version,
        )?;
        Ok(self.runner.get_or_init(|| runner))
    }
}

impl PublisherType for Cargo {
    const NAME: &'static str = "cargo";

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: CargoDeclaration = shared::decode(declaration)?;
        declaration.validate()?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

fn describe(index: &str, name: &str, version: &Version) -> Description {
    Description {
        target: index.into(),
        items: vec![("crate".into(), format!("{name}@{version}"))],
    }
}

/// What the index lists for `version` of `name`.
fn lookup(
    registry: &impl CargoRegistry,
    name: &str,
    version: &Version,
) -> Result<Option<IndexEntry>, PublicationError> {
    Ok(registry.versions(name)?.and_then(|entries| {
        entries
            .into_iter()
            .find(|entry| Version::parse(&entry.vers).is_ok_and(|vers| vers == *version))
    }))
}

/// Whether the index lists the crate `name` of `release`. Without the archive its checksum cannot
/// be compared; a publish compares it. An index that cannot be read makes the release `Unknown`.
pub fn completeness_with(
    registry: &impl CargoRegistry,
    index: &str,
    name: &str,
    release: &Release,
) -> Completeness {
    match lookup(registry, name, &release.version) {
        Ok(Some(_)) => Completeness::Complete,
        Ok(None) => Completeness::Incomplete {
            target: format!("crate {name}@{} on {index}", release.version),
            reason: "the index does not list it".into(),
        },
        Err(error) => Completeness::Unknown(error.to_string()),
    }
}

impl Publisher for Cargo {
    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        vec![("token", &self.declaration.token)]
    }

    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        Ok(describe(
            &self.declaration.index,
            &self.declaration.name,
            version,
        ))
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        vec![("crate".into(), self.declaration.archive.clone())]
    }

    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        let declaration = &self.declaration;
        // Most indexes are public; one that needs authentication reads with the token if it is set.
        let token = ctx.session.secrets.get(&declaration.token).ok();
        let registry = HttpRegistry::new(&declaration.index, token)?;
        Ok(completeness_with(
            &registry,
            &declaration.index,
            &declaration.name,
            release,
        ))
    }

    fn state(&self, release: &Release, ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.runner(release, ctx)?.state(release, ctx)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        self.runner(release, ctx)?.publish(release, ctx)
    }

    fn complete_steps(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        self.runner(release, ctx)?.complete_steps(release, ctx)
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

/// How long a publisher waits for the index to list the version it uploaded.
#[derive(Debug, Clone, Copy)]
pub struct IndexWait {
    pub timeout: Duration,
    pub interval: Duration,
}

impl Default for IndexWait {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            interval: Duration::from_secs(2),
        }
    }
}

/// A Cargo publisher with its own registry client, such as a fake one in tests.
pub struct CargoPublisher<R> {
    registry: R,
    index: String,
    local: LocalCrate,
    wait: IndexWait,
}

impl<R: CargoRegistry> CargoPublisher<R> {
    /// Reads the archive at `archive` and checks that it is version `release_version` of the
    /// crate `name`, the two things the publish request reads from it anyway.
    pub fn new(
        registry: R,
        index: &str,
        name: &str,
        archive: &str,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        let local = LocalCrate::open(archive, index, release_version)?;
        if local.name != name {
            return Err(PublicationError::new(format!(
                "crate archive {archive:?} holds crate {:?}, expected name {name:?}",
                local.name
            )));
        }
        Ok(Self {
            registry,
            index: index.into(),
            local,
            wait: IndexWait::default(),
        })
    }

    #[must_use]
    pub fn with_wait(mut self, wait: IndexWait) -> Self {
        self.wait = wait;
        self
    }

    fn reference(&self) -> String {
        format!("{}@{}", self.local.name, self.local.version)
    }

    fn step(&self, step: AssetStep) -> ArtifactSteps {
        vec![ArtifactStep {
            kind: CRATE,
            name: self.reference(),
            step,
        }]
    }

    /// The remote state of one index entry compared with the local archive.
    fn compare(&self, entry: Option<&IndexEntry>) -> PublisherState {
        match entry {
            None => PublisherState::Missing,
            Some(entry) if entry.cksum == self.local.cksum => PublisherState::Published,
            Some(entry) => PublisherState::Conflicting(format!(
                "{} on {} has checksum {}, expected {} of {}",
                self.reference(),
                self.index,
                entry.cksum,
                self.local.cksum,
                self.local.origin
            )),
        }
    }

    /// The remote state, and whether the listed version is yanked.
    fn inspect(&self) -> Result<(PublisherState, bool), PublicationError> {
        let entry = lookup(&self.registry, &self.local.name, &self.local.version)?;
        let yanked = entry.as_ref().is_some_and(|entry| entry.yanked);
        Ok((self.compare(entry.as_ref()), yanked))
    }

    /// The step of a version the index already lists: a yanked one stays yanked.
    fn already_uploaded(&self, yanked: bool) -> ArtifactSteps {
        self.step(if yanked {
            AssetStep::Kept("yanked on the index".into())
        } else {
            AssetStep::AlreadyUploaded
        })
    }

    /// Waits until the index lists the version this run uploaded, so publishers after this one
    /// find it.
    fn wait_for_index(&self) -> Result<(), PublicationError> {
        let start = Instant::now();
        loop {
            let entry = lookup(&self.registry, &self.local.name, &self.local.version)?;
            match self.compare(entry.as_ref()) {
                PublisherState::Published => return Ok(()),
                PublisherState::Conflicting(message) => return Err(PublicationError::new(message)),
                _ if start.elapsed() >= self.wait.timeout => {
                    return Err(PublicationError::new(format!(
                        "{} was uploaded, but {} did not list it within {} seconds; rerun publish once it does",
                        self.reference(),
                        self.index,
                        self.wait.timeout.as_secs()
                    )));
                }
                _ => std::thread::sleep(self.wait.interval),
            }
        }
    }

    fn upload(&self, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        match self.inspect()? {
            (PublisherState::Missing, _) => {}
            (PublisherState::Conflicting(message), _) => {
                return Err(PublicationError::new(message));
            }
            (_, yanked) => return Ok(self.already_uploaded(yanked)),
        }
        let warnings = match self.registry.upload(self.local.publish_body()) {
            Ok(warnings) => warnings,
            // A run at the same time may have uploaded it: inspect again instead of failing.
            Err(error) => {
                return match self.inspect()? {
                    (PublisherState::Published, yanked) => Ok(self.already_uploaded(yanked)),
                    _ => Err(error),
                };
            }
        };
        for warning in warnings {
            ctx.warn(
                WarningLevel::Warning,
                format!("{}: {warning}", self.reference()),
            );
        }
        self.wait_for_index()?;
        Ok(self.step(AssetStep::Uploaded))
    }
}

impl<R: CargoRegistry> Publisher for CargoPublisher<R> {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        Ok(describe(&self.index, &self.local.name, version))
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        Ok(completeness_with(
            &self.registry,
            &self.index,
            &self.local.name,
            release,
        ))
    }

    fn state(
        &self,
        _release: &Release,
        _ctx: &Context,
    ) -> Result<PublisherState, PublicationError> {
        Ok(self.inspect()?.0)
    }

    fn publish(
        &self,
        _release: &Release,
        ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        let steps = self.upload(ctx)?;
        steps.iter().for_each(|step| ctx.progress(step));
        Ok(steps)
    }

    fn complete_steps(
        &self,
        _release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        Ok(self.already_uploaded(self.inspect()?.1))
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

#[cfg(test)]
mod tests {
    use super::package::tests::{archive, manifest};
    use super::*;
    use crate::publication::{PublisherNode, RemoteState, TagPublisher, publish_ordered};
    use crate::publishers::Warning;
    use crate::publishers::shared::digest::sha256_digest;
    use serde_json::json;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    const INDEX: &str = "sparse+https://index.crates.io/";

    #[derive(Default)]
    struct FakeState {
        entries: HashMap<String, Vec<IndexEntry>>,
        uploads: Vec<Vec<u8>>,
        /// How many index reads after an upload still miss it, like a lagging CDN.
        lag: usize,
        pending: Option<(String, IndexEntry)>,
        /// Uploads fail, as when another run uploaded the same version first.
        upload_fails: bool,
        index_fails: bool,
    }

    #[derive(Default, Clone)]
    struct FakeRegistry(Arc<Mutex<FakeState>>);

    impl FakeRegistry {
        fn with(name: &str, entry: IndexEntry) -> Self {
            let registry = Self::default();
            registry
                .0
                .lock()
                .unwrap()
                .entries
                .insert(name.into(), vec![entry]);
            registry
        }
    }

    impl CargoRegistry for FakeRegistry {
        fn versions(&self, name: &str) -> Result<Option<Vec<IndexEntry>>, PublicationError> {
            let mut state = self.0.lock().unwrap();
            if state.index_fails {
                return Err(PublicationError::new("could not read the index (HTTP 500)"));
            }
            if state.pending.is_some() {
                if state.lag == 0 {
                    let (name, entry) = state.pending.take().unwrap();
                    state.entries.entry(name).or_default().push(entry);
                } else {
                    state.lag -= 1;
                }
            }
            Ok(state.entries.get(name).cloned())
        }

        fn upload(&self, body: Vec<u8>) -> Result<Vec<String>, PublicationError> {
            let mut state = self.0.lock().unwrap();
            if state.upload_fails {
                return Err(PublicationError::new(
                    "could not publish (HTTP 400): crate version is already uploaded",
                ));
            }
            let length = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
            let metadata: Value = serde_json::from_slice(&body[4..4 + length]).unwrap();
            let archive = &body[8 + length..];
            let name = metadata["name"].as_str().unwrap().to_owned();
            state.pending = Some((
                name,
                IndexEntry {
                    vers: metadata["vers"].as_str().unwrap().into(),
                    cksum: cksum(archive),
                    yanked: false,
                },
            ));
            state.uploads.push(body);
            Ok(vec!["invalid categories: nope".into()])
        }
    }

    fn cksum(bytes: &[u8]) -> String {
        sha256_digest(bytes).trim_start_matches("sha256:").into()
    }

    fn write_crate(directory: &Path, name: &str, version: &str) -> (PathBuf, Vec<u8>) {
        let manifest = manifest(name, version);
        let bytes = archive(
            name,
            version,
            &[("Cargo.toml", &manifest), ("README.md", "# Demo\n")],
        );
        let path = directory.join(format!("{name}-{version}.crate"));
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    fn release(version: &str) -> Release {
        Release {
            commit: "0123456789012345678901234567890123456789".into(),
            version: Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            notes: String::new(),
        }
    }

    fn publisher(registry: &FakeRegistry, path: &Path) -> CargoPublisher<FakeRegistry> {
        CargoPublisher::new(
            registry.clone(),
            INDEX,
            "demo",
            &path.to_string_lossy(),
            &Version::new(1, 2, 3),
        )
        .unwrap()
        .with_wait(IndexWait {
            timeout: Duration::from_millis(200),
            interval: Duration::from_millis(1),
        })
    }

    fn entry(cksum: &str, yanked: bool) -> IndexEntry {
        IndexEntry {
            vers: "1.2.3".into(),
            cksum: cksum.into(),
            yanked,
        }
    }

    #[test]
    fn uploads_a_missing_crate_waits_for_the_index_and_a_retry_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        // A derivation output that holds the archive.
        let (_, bytes) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::default();
        registry.0.lock().unwrap().lag = 3;
        let publisher = publisher(&registry, directory.path());
        let release = release("1.2.3");
        let warnings = Mutex::new(Vec::new());
        let warn = |warning: Warning| warnings.lock().unwrap().push(warning.to_string());
        let ctx = Context::detached().with_warnings(&warn);

        assert_eq!(
            publisher.state(&release, &ctx).unwrap(),
            PublisherState::Missing
        );
        assert_eq!(
            publisher.publish(&release, &ctx).unwrap(),
            [ArtifactStep {
                kind: CRATE,
                name: "demo@1.2.3".into(),
                step: AssetStep::Uploaded,
            }]
        );
        // The publish returned only once the index listed the upload.
        assert_eq!(
            publisher.state(&release, &ctx).unwrap(),
            PublisherState::Published
        );
        assert_eq!(
            *warnings.lock().unwrap(),
            ["warning: demo@1.2.3: invalid categories: nope"]
        );

        let uploads = registry.0.lock().unwrap().uploads.clone();
        assert_eq!(uploads.len(), 1);
        assert!(uploads[0].ends_with(&bytes));

        assert_eq!(
            publisher.publish(&release, &ctx).unwrap()[0].step,
            AssetStep::AlreadyUploaded
        );
        assert_eq!(registry.0.lock().unwrap().uploads.len(), 1);
    }

    #[test]
    fn an_index_that_never_lists_the_upload_fails_after_the_bounded_wait() {
        let directory = tempfile::tempdir().unwrap();
        let (path, _) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::default();
        registry.0.lock().unwrap().lag = usize::MAX;

        let error = publisher(&registry, &path)
            .publish(&release("1.2.3"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert!(error.contains("demo@1.2.3 was uploaded, but"), "{error}");
    }

    #[test]
    fn a_matching_version_is_published_even_when_yanked_and_is_not_waited_for() {
        let directory = tempfile::tempdir().unwrap();
        let (path, bytes) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::with("demo", entry(&cksum(&bytes), true));
        let ctx = Context::detached();
        let publisher = publisher(&registry, &path);
        let kept = AssetStep::Kept("yanked on the index".into());

        assert_eq!(
            publisher.state(&release("1.2.3"), &ctx).unwrap(),
            PublisherState::Published
        );
        assert_eq!(
            publisher.publish(&release("1.2.3"), &ctx).unwrap()[0].step,
            kept
        );
        assert_eq!(
            publisher.complete_steps(&release("1.2.3"), &ctx).unwrap()[0].step,
            kept
        );
        assert!(registry.0.lock().unwrap().uploads.is_empty());
    }

    struct FakeTagPublisher {
        writes: Rc<Cell<usize>>,
    }

    impl TagPublisher for FakeTagPublisher {
        fn state(&self, _release: &Release) -> Result<RemoteState, PublicationError> {
            Ok(RemoteState::Missing)
        }

        fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
            self.writes.set(self.writes.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn another_checksum_conflicts_before_the_git_tag_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let (path, _) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::with("demo", entry("0000", false));
        let tag_writes = Rc::new(Cell::new(0));

        let error = publish_ordered(
            &FakeTagPublisher {
                writes: tag_writes.clone(),
            },
            &release("1.2.3"),
            &[PublisherNode::new(
                "cargo.demo",
                Box::new(publisher(&registry, &path)),
                Vec::new(),
            )],
            false,
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("has checksum 0000"), "{error}");
        assert_eq!(tag_writes.get(), 0);
        assert!(registry.0.lock().unwrap().uploads.is_empty());
    }

    #[test]
    fn a_failed_upload_is_inspected_again_before_it_fails() {
        let directory = tempfile::tempdir().unwrap();
        let (path, bytes) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::default();
        let publisher = publisher(&registry, &path);
        registry.0.lock().unwrap().upload_fails = true;

        let error = publisher
            .publish(&release("1.2.3"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert!(error.contains("already uploaded"), "{error}");

        // Another run uploads the same archive between this run's state check and its upload.
        let mut state = registry.0.lock().unwrap();
        state.pending = Some(("demo".into(), entry(&cksum(&bytes), false)));
        state.lag = 1;
        drop(state);
        assert_eq!(
            publisher
                .publish(&release("1.2.3"), &Context::detached())
                .unwrap()[0]
                .step,
            AssetStep::AlreadyUploaded
        );
    }

    #[test]
    fn an_unreadable_index_fails_closed_and_makes_completeness_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let (path, _) = write_crate(directory.path(), "demo", "1.2.3");
        let registry = FakeRegistry::default();
        let publisher = publisher(&registry, &path);
        registry.0.lock().unwrap().index_fails = true;

        let error = publisher
            .state(&release("1.2.3"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP 500"), "{error}");
        assert!(matches!(
            completeness_with(&registry, INDEX, "demo", &release("1.2.3")),
            Completeness::Unknown(_)
        ));
        assert!(registry.0.lock().unwrap().uploads.is_empty());
    }

    #[test]
    fn completeness_needs_only_the_index_entry() {
        let registry = FakeRegistry::with("demo", entry("any", true));
        assert_eq!(
            completeness_with(&registry, INDEX, "demo", &release("1.2.3")),
            Completeness::Complete
        );
        assert_eq!(
            completeness_with(&registry, INDEX, "demo", &release("1.2.4")),
            Completeness::Incomplete {
                target: format!("crate demo@1.2.4 on {INDEX}"),
                reason: "the index does not list it".into(),
            }
        );
    }

    #[test]
    fn the_archive_must_hold_the_declared_crate() {
        let directory = tempfile::tempdir().unwrap();
        let (path, _) = write_crate(directory.path(), "other", "1.2.3");
        let registry = FakeRegistry::default();
        let error = CargoPublisher::new(
            registry,
            INDEX,
            "demo",
            &path.to_string_lossy(),
            &Version::new(1, 2, 3),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            error.contains("holds crate \"other\", expected name \"demo\""),
            "{error}"
        );
    }

    #[test]
    fn declarations_are_validated() {
        let declaration = |index: &str, token: Value| {
            json!({
                "name": "demo",
                "crate": "/nix/store/00000000000000000000000000000000-demo",
                "index": index,
                "token": token,
            })
        };
        let cargo =
            Cargo::from_declaration(&declaration(INDEX, json!({ "env": "TOKEN" }))).unwrap();
        assert_eq!(
            cargo.describe(&Version::new(1, 2, 3), "v1.2.3").unwrap(),
            Description {
                target: INDEX.into(),
                items: vec![("crate".into(), "demo@1.2.3".into())],
            }
        );
        assert_eq!(cargo.credentials(), [("token", &Credential::env("TOKEN"))]);

        for (index, token) in [
            ("https://index.crates.io/", json!({ "env": "TOKEN" })),
            ("sparse+https://index.crates.io", json!({ "env": "TOKEN" })),
            (
                "sparse+https://user:pw@kellnr.example/api/v1/crates/",
                json!({ "env": "TOKEN" }),
            ),
            ("sparse+ftp://index.example/", json!({ "env": "TOKEN" })),
            (INDEX, json!({ "value": "plain" })),
        ] {
            assert!(
                Cargo::from_declaration(&declaration(index, token.clone())).is_err(),
                "{index} {token}"
            );
        }
        let mut bad_name = declaration(INDEX, json!({ "env": "TOKEN" }));
        bad_name["name"] = json!("../demo");
        assert!(Cargo::from_declaration(&bad_name).is_err());
    }
}
