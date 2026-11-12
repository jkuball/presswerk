use presswerk::oci::{
    Credentials, INDEX_MEDIA_TYPE, LAYER_MEDIA_TYPE, LocalImage, OciPublisher, RegistryClient,
    image_index,
};
use presswerk::publication::{
    ArtifactKind, ArtifactStep, AssetStep, Publisher, PublisherState, Release,
};
use semver::Version;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, contents) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(1);
        builder.append_data(&mut header, name, *contents).unwrap();
    }
    builder.into_inner().unwrap()
}

/// A Docker image archive like the ones `dockerTools` writes, with uncompressed layers.
fn docker_archive(architecture: &str, contents: &str) -> Vec<u8> {
    let first = tar(&[("etc/greeting", contents.as_bytes())]);
    let second = tar(&[("bin/tool", architecture.as_bytes())]);
    let config = serde_json::to_vec(&json!({
        "architecture": architecture,
        "os": "linux",
        "config": { "Cmd": ["/bin/tool"] },
        "rootfs": { "type": "layers", "diff_ids": [digest(&first), digest(&second)] },
    }))
    .unwrap();
    let config_name = format!("{}.json", &digest(&config)[7..]);
    let manifest = serde_json::to_vec(&json!([{
        "Config": config_name,
        "RepoTags": ["image:latest"],
        "Layers": ["first/layer.tar", "second/layer.tar"],
    }]))
    .unwrap();
    tar(&[
        ("manifest.json", &manifest),
        (&config_name, &config),
        ("first/layer.tar", &first),
        ("second/layer.tar", &second),
    ])
}

fn write(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A script that streams an archive, like the output of `dockerTools.streamLayeredImage`.
fn stream_script(directory: &Path, name: &str, archive: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = write(
        directory,
        name,
        format!("#!/bin/sh\nexec cat '{}'\n", archive.display()).as_bytes(),
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn every_dockertools_format_converts_to_the_same_uncompressed_oci_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let archive = docker_archive("amd64", "hello");
    let plain = write(directory.path(), "image.tar", &archive);
    let gzipped = write(directory.path(), "image.tar.gz", &gzip(&archive));
    let streamed = stream_script(directory.path(), "stream-image", &plain);

    let images: Vec<_> = [&plain, &gzipped, &streamed]
        .into_iter()
        .map(|path| LocalImage::load("linux/amd64", path).unwrap())
        .collect();
    assert!(
        images
            .iter()
            .all(|image| image.manifest_digest == images[0].manifest_digest)
    );
    assert_eq!(images[0].manifest_digest, digest(&images[0].manifest));

    let manifest: Value = serde_json::from_slice(&images[0].manifest).unwrap();
    assert_eq!(
        manifest["mediaType"],
        "application/vnd.oci.image.manifest.v1+json"
    );
    assert_eq!(
        manifest["config"]["mediaType"],
        "application/vnd.oci.image.config.v1+json"
    );
    let layers = manifest["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert!(
        layers
            .iter()
            .all(|layer| layer["mediaType"] == LAYER_MEDIA_TYPE)
    );
    // Uncompressed layers: the layer digest is the config's diff_id.
    assert_eq!(layers[0]["digest"], images[0].layers[0].digest);
}

#[test]
fn an_image_must_match_its_declared_platform_and_carry_uncompressed_layers() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        directory.path(),
        "image.tar",
        &docker_archive("arm64", "hello"),
    );
    let error = LocalImage::load("linux/amd64", &path).unwrap_err();
    assert!(error.contains("built for linux/arm64"), "{error}");

    // An archive whose layer does not hash to its diff_id, such as a compressed layer.
    let layer = gzip(&tar(&[("file", b"x")]));
    let config = serde_json::to_vec(&json!({
        "architecture": "amd64",
        "os": "linux",
        "rootfs": { "type": "layers", "diff_ids": [digest(&tar(&[("file", b"x")]))] },
    }))
    .unwrap();
    let manifest =
        serde_json::to_vec(&json!([{ "Config": "config.json", "Layers": ["layer.tar"] }])).unwrap();
    let path = write(
        directory.path(),
        "compressed.tar",
        &tar(&[
            ("manifest.json", &manifest),
            ("config.json", &config),
            ("layer.tar", &layer),
        ]),
    );
    let error = LocalImage::load("linux/amd64", &path).unwrap_err();
    assert!(error.contains("uncompressed layers"), "{error}");

    let directory_image = LocalImage::load("linux/amd64", directory.path()).unwrap_err();
    assert!(directory_image.contains("not a file"), "{directory_image}");

    let failing = write(directory.path(), "failing", b"#!/bin/sh\nexit 3\n");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(LocalImage::load("linux/amd64", &failing).is_err());
}

#[test]
fn the_image_index_is_reproducible_sorted_and_annotated_with_the_version() {
    let directory = tempfile::tempdir().unwrap();
    let load = |platform: &str, architecture: &str| {
        let path = write(
            directory.path(),
            architecture,
            &docker_archive(architecture, "hello"),
        );
        LocalImage::load(platform, path).unwrap()
    };
    let arm = load("linux/arm64", "arm64");
    let amd = load("linux/amd64", "amd64");
    let version = Version::new(1, 2, 0);
    let images = [arm, amd];
    let (index, index_digest) = image_index(&images, &version);
    let again = image_index(&images, &version);
    assert_eq!((index.clone(), index_digest.clone()), again);
    assert_eq!(index_digest, digest(&index));

    let index: Value = serde_json::from_slice(&index).unwrap();
    assert_eq!(index["mediaType"], INDEX_MEDIA_TYPE);
    assert_eq!(
        index["annotations"]["org.opencontainers.image.version"],
        "1.2.0"
    );
    let platforms: Vec<_> = index["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|manifest| manifest["platform"]["architecture"].as_str().unwrap())
        .collect();
    assert_eq!(platforms, ["amd64", "arm64"]);

    assert_ne!(
        image_index(&images, &Version::new(1, 2, 1)).1,
        index_digest,
        "each version gets its own index"
    );
}

/// What the mock registry does to the next matching request instead of answering normally.
#[derive(Clone)]
enum Fault {
    /// Store the write, then answer 500, like a response lost on the way back.
    LoseResponse(&'static str),
    /// Point the tag at another digest before answering 500, like a concurrent writer.
    HijackTag(&'static str),
    /// Answer every inspection of a manifest with this status.
    InspectStatus(u16),
    /// Answer a write of this tag with 500 without storing it.
    RejectTag(&'static str),
    /// Answer every inspection of this tag with 500.
    UnreadableTag(&'static str),
}

#[derive(Default)]
struct RegistryState {
    blobs: HashMap<String, Vec<u8>>,
    manifests: HashMap<String, (String, Vec<u8>)>,
    tags: HashMap<String, String>,
    writes: Vec<String>,
    faults: Vec<Fault>,
    uploads: usize,
    /// Whether anyone may pull without credentials.
    public: bool,
    /// The query and `Authorization` header of every token request.
    token_requests: Vec<(String, Option<String>)>,
}

struct MockRegistry {
    address: String,
    state: Arc<Mutex<RegistryState>>,
}

impl MockRegistry {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(RegistryState::default()));
        let shared = state.clone();
        let realm = format!("http://{address}/token");
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let state = shared.clone();
                let realm = realm.clone();
                thread::spawn(move || handle(stream, &state, &realm));
            }
        });
        Self { address, state }
    }

    fn repository(&self) -> String {
        format!("{}/team/app", self.address)
    }

    fn publisher(&self, images: Vec<LocalImage>, tags: &[&str]) -> OciPublisher {
        self.publisher_with(images, tags, "secret-password")
    }

    fn publisher_with(
        &self,
        images: Vec<LocalImage>,
        tags: &[&str],
        password: &str,
    ) -> OciPublisher {
        let credentials = Credentials {
            username: "user".into(),
            password: password.into(),
        };
        OciPublisher::new(
            RegistryClient::new(&self.repository(), Some(credentials)).unwrap(),
            &self.repository(),
            tags.iter().map(|tag| (*tag).into()).collect(),
            images,
        )
    }

    fn anonymous_client(&self) -> RegistryClient {
        RegistryClient::new(&self.repository(), None).unwrap()
    }

    fn writes(&self) -> Vec<String> {
        self.state.lock().unwrap().writes.clone()
    }

    fn tag(&self, tag: &str) -> Option<String> {
        self.state.lock().unwrap().tags.get(tag).cloned()
    }

    fn set_tag(&self, tag: &str, digest: &str) {
        self.state
            .lock()
            .unwrap()
            .tags
            .insert(tag.into(), digest.into());
    }

    fn fault(&self, fault: Fault) {
        self.state.lock().unwrap().faults.push(fault);
    }
}

fn respond(stream: &mut TcpStream, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut response = format!(
        "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn handle(mut stream: TcpStream, state: &Mutex<RegistryState>, realm: &str) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap().to_owned();
    let target = parts.next().unwrap().to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .map_or(0, |length| length.parse().unwrap());
    assert!(
        !headers.contains_key("transfer-encoding"),
        "uploads must declare their length"
    );
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();

    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    if path == "/token" {
        let mut state = state.lock().unwrap();
        let authorization = headers.get("authorization").cloned();
        state
            .token_requests
            .push((query.to_owned(), authorization.clone()));
        match authorization.as_deref() {
            // "user:secret-password"
            Some("Basic dXNlcjpzZWNyZXQtcGFzc3dvcmQ=") => {
                assert!(
                    query.contains("scope=repository%3Ateam%2Fapp%3Apull%2Cpush"),
                    "{query}"
                );
                respond(&mut stream, 200, &[], br#"{"token":"test-token"}"#);
            }
            None if state.public => {
                respond(&mut stream, 200, &[], br#"{"token":"anonymous-token"}"#);
            }
            _ => respond(&mut stream, 401, &[], b"{}"),
        }
        return;
    }
    let anonymous_read = headers.get("authorization").map(String::as_str)
        == Some("Bearer anonymous-token")
        && matches!(method.as_str(), "GET" | "HEAD");
    if headers.get("authorization").map(String::as_str) != Some("Bearer test-token")
        && !anonymous_read
    {
        respond(
            &mut stream,
            401,
            &[(
                "WWW-Authenticate",
                format!(
                    r#"Bearer realm="{realm}",service="mock",scope="repository:team/app:pull""#
                ),
            )],
            b"",
        );
        return;
    }

    let path = path.strip_prefix("/v2/team/app/").expect("repository path");
    let mut state = state.lock().unwrap();
    let state = &mut *state;
    if !matches!(method.as_str(), "GET" | "HEAD") {
        state.writes.push(format!("{method} {path}"));
    }
    let fault = state.faults.first().cloned();
    let take_fault = |state: &mut RegistryState| {
        state.faults.remove(0);
    };

    if let Some(reference) = path.strip_prefix("manifests/") {
        match method.as_str() {
            "GET" | "HEAD" => {
                if let Some(Fault::InspectStatus(status)) = fault {
                    respond(&mut stream, status, &[], b"");
                    return;
                }
                if let Some(Fault::UnreadableTag(tag)) = fault
                    && tag == reference
                {
                    respond(&mut stream, 500, &[], b"");
                    return;
                }
                let digest = state
                    .tags
                    .get(reference)
                    .cloned()
                    .unwrap_or_else(|| reference.to_owned());
                match state.manifests.get(&digest) {
                    Some((media_type, bytes)) => {
                        let headers = [
                            ("Docker-Content-Digest", digest.clone()),
                            ("Content-Type", media_type.clone()),
                        ];
                        if method == "HEAD" {
                            let mut response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n",
                                bytes.len()
                            );
                            for (name, value) in headers {
                                response.push_str(&format!("{name}: {value}\r\n"));
                            }
                            response.push_str("\r\n");
                            stream.write_all(response.as_bytes()).unwrap();
                        } else {
                            respond(&mut stream, 200, &headers, bytes);
                        }
                    }
                    None => respond(&mut stream, 404, &[], b""),
                }
            }
            "PUT" => {
                let digest = digest(&body);
                if let Some(Fault::RejectTag(tag)) = fault
                    && tag == reference
                {
                    take_fault(state);
                    respond(&mut stream, 500, &[], b"");
                    return;
                }
                if let Some(Fault::HijackTag(tag)) = fault
                    && tag == reference
                {
                    take_fault(state);
                    state.tags.insert(tag.into(), "sha256:hijacked".into());
                    respond(&mut stream, 500, &[], b"");
                    return;
                }
                let manifest: Value = serde_json::from_slice(&body).unwrap();
                let blobs_known = manifest["layers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(
                        [&manifest["config"]]
                            .into_iter()
                            .filter(|config| !config.is_null()),
                    )
                    .all(|descriptor| {
                        state
                            .blobs
                            .contains_key(descriptor["digest"].as_str().unwrap())
                    });
                let manifests_known =
                    manifest["manifests"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .all(|descriptor| {
                            state
                                .manifests
                                .contains_key(descriptor["digest"].as_str().unwrap())
                        });
                if !blobs_known
                    || !manifests_known
                    || (reference.starts_with("sha256:") && reference != digest)
                {
                    respond(&mut stream, 400, &[], b"");
                    return;
                }
                state
                    .manifests
                    .insert(digest.clone(), (headers["content-type"].clone(), body));
                if !reference.starts_with("sha256:") {
                    state.tags.insert(reference.into(), digest.clone());
                }
                if let Some(Fault::LoseResponse(target)) = fault
                    && target == reference
                {
                    take_fault(state);
                    respond(&mut stream, 500, &[], b"");
                    return;
                }
                respond(&mut stream, 201, &[("Docker-Content-Digest", digest)], b"");
            }
            _ => respond(&mut stream, 405, &[], b""),
        }
    } else if let Some(blob) = path.strip_prefix("blobs/uploads/") {
        match method.as_str() {
            "POST" => {
                state.uploads += 1;
                let location = format!("/v2/team/app/blobs/uploads/{}?state=opaque", state.uploads);
                respond(&mut stream, 202, &[("Location", location)], b"");
            }
            "PUT" => {
                assert!(!blob.is_empty());
                assert!(query.starts_with("state=opaque&digest="), "{query}");
                let expected = query.split("digest=").nth(1).unwrap().replace("%3A", ":");
                if digest(&body) != expected {
                    respond(&mut stream, 400, &[], b"");
                    return;
                }
                state.blobs.insert(expected.clone(), body);
                if let Some(Fault::LoseResponse(target)) = fault
                    && target == expected
                {
                    take_fault(state);
                    respond(&mut stream, 500, &[], b"");
                    return;
                }
                respond(&mut stream, 201, &[], b"");
            }
            _ => respond(&mut stream, 405, &[], b""),
        }
    } else if let Some(digest) = path.strip_prefix("blobs/") {
        let status = if state.blobs.contains_key(digest) {
            200
        } else {
            404
        };
        respond(&mut stream, status, &[], b"");
    } else {
        respond(&mut stream, 404, &[], b"");
    }
}

fn release(version: &str) -> Release {
    let version = Version::parse(version).unwrap();
    Release {
        commit: "0123456789012345678901234567890123456789".into(),
        tag: format!("v{version}"),
        version,
        notes: String::new(),
    }
}

struct Images {
    _directory: tempfile::TempDir,
    paths: Vec<(String, PathBuf)>,
}

impl Images {
    fn new(contents: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let amd = write(
            directory.path(),
            "amd64.tar.gz",
            &gzip(&docker_archive("amd64", contents)),
        );
        let arm_archive = write(
            directory.path(),
            "arm64.tar",
            &docker_archive("arm64", contents),
        );
        let arm = stream_script(directory.path(), "arm64-stream", &arm_archive);
        Self {
            _directory: directory,
            paths: vec![("linux/amd64".into(), amd), ("linux/arm64".into(), arm)],
        }
    }

    fn load(&self) -> Vec<LocalImage> {
        self.paths
            .iter()
            .map(|(platform, path)| LocalImage::load(platform, path).unwrap())
            .collect()
    }
}

fn step(kind: ArtifactKind, name: &str, step: AssetStep) -> ArtifactStep {
    ArtifactStep {
        kind,
        name: name.into(),
        step,
    }
}

#[test]
fn pushes_a_new_release_and_a_rerun_writes_nothing() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let repository = registry.repository();
    let publisher = registry.publisher(images.load(), &["{version}", "latest"]);
    let release = release("1.0.0");

    assert_eq!(publisher.state(&release).unwrap(), PublisherState::Missing);
    let steps = publisher.publish(&release).unwrap();
    assert_eq!(
        steps,
        [
            step(ArtifactKind::Image, "linux/amd64", AssetStep::Uploaded),
            step(ArtifactKind::Image, "linux/arm64", AssetStep::Uploaded),
            step(
                ArtifactKind::Tag,
                &format!("{repository}:1.0.0"),
                AssetStep::Uploaded
            ),
            step(
                ArtifactKind::Alias,
                &format!("{repository}:latest"),
                AssetStep::Uploaded
            ),
        ]
    );
    let (_, index_digest) = image_index(&images.load(), &release.version);
    assert_eq!(registry.tag("1.0.0"), Some(index_digest.clone()));
    assert_eq!(registry.tag("latest"), Some(index_digest));
    assert!(
        registry
            .writes()
            .iter()
            .all(|write| !write.contains("manifests/")
                || write.ends_with("1.0.0")
                || write.ends_with("latest")
                || write.contains("sha256:")),
    );

    // A retry with freshly loaded images reproduces the same digests and writes nothing.
    let writes = registry.writes().len();
    let rerun = registry.publisher(images.load(), &["{version}", "latest"]);
    assert_eq!(rerun.state(&release).unwrap(), PublisherState::Published);
    assert_eq!(
        rerun.complete_steps(&release).unwrap(),
        [
            step(
                ArtifactKind::Image,
                "linux/amd64",
                AssetStep::AlreadyUploaded
            ),
            step(
                ArtifactKind::Image,
                "linux/arm64",
                AssetStep::AlreadyUploaded
            ),
            step(
                ArtifactKind::Tag,
                &format!("{repository}:1.0.0"),
                AssetStep::AlreadyUploaded
            ),
            step(
                ArtifactKind::Alias,
                &format!("{repository}:latest"),
                AssetStep::AlreadyUploaded
            ),
        ]
    );
    rerun.publish(&release).unwrap();
    assert_eq!(registry.writes().len(), writes);
}

#[test]
fn a_deleted_tag_is_restored_without_uploading_blobs_again() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let release = release("1.0.0");
    registry
        .publisher(images.load(), &["{version}"])
        .publish(&release)
        .unwrap();
    let digest = registry.tag("1.0.0").unwrap();
    registry.state.lock().unwrap().tags.remove("1.0.0");
    let writes = registry.writes().len();

    let publisher = registry.publisher(images.load(), &["{version}"]);
    assert_eq!(publisher.state(&release).unwrap(), PublisherState::Missing);
    let steps = publisher.publish(&release).unwrap();
    assert_eq!(steps[0].step, AssetStep::AlreadyUploaded);
    assert_eq!(steps[2].step, AssetStep::Uploaded);
    assert_eq!(registry.writes()[writes..], ["PUT manifests/1.0.0"]);
    assert_eq!(registry.tag("1.0.0"), Some(digest));
}

#[test]
fn a_tag_that_points_elsewhere_conflicts_before_anything_is_written() {
    let registry = MockRegistry::start();
    let release = release("1.0.0");
    let other = Images::new("other");
    registry
        .publisher(other.load(), &["{version}"])
        .publish(&release)
        .unwrap();
    let other_digest = registry.tag("1.0.0").unwrap();
    let writes = registry.writes().len();

    let images = Images::new("hello");
    let publisher = registry.publisher(images.load(), &["{version}", "latest"]);
    let PublisherState::Conflicting(message) = publisher.state(&release).unwrap() else {
        panic!("expected a conflict");
    };
    assert!(
        message.contains(&format!("points to {other_digest}")),
        "{message}"
    );
    let error = publisher.publish(&release).unwrap_err().to_string();
    assert!(error.contains("points to"), "{error}");
    assert_eq!(registry.writes().len(), writes);
    assert_eq!(registry.tag("1.0.0"), Some(other_digest));
}

#[test]
fn an_unknown_tag_state_fails_closed() {
    let registry = MockRegistry::start();
    registry.fault(Fault::InspectStatus(500));
    let images = Images::new("hello");
    let publisher = registry.publisher(images.load(), &["{version}"]);
    let error = publisher.state(&release("1.0.0")).unwrap_err().to_string();
    assert!(error.contains("cannot tell"), "{error}");
    assert!(publisher.publish(&release("1.0.0")).is_err());
    assert!(registry.writes().is_empty());
}

#[test]
fn lost_responses_are_inspected_again_and_accepted_only_when_matching() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let loaded = images.load();
    let layer = loaded[0].layers[0].digest.clone();
    registry.fault(Fault::LoseResponse(Box::leak(layer.into_boxed_str())));
    registry.fault(Fault::LoseResponse("1.0.0"));
    registry
        .publisher(loaded, &["{version}"])
        .publish(&release("1.0.0"))
        .unwrap();
    assert!(registry.state.lock().unwrap().faults.is_empty());
}

#[test]
fn a_tag_written_concurrently_to_another_image_is_a_conflict() {
    let registry = MockRegistry::start();
    registry.fault(Fault::HijackTag("1.0.0"));
    let images = Images::new("hello");
    let error = registry
        .publisher(images.load(), &["{version}"])
        .publish(&release("1.0.0"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("could not push"), "{error}");
    assert_eq!(registry.tag("1.0.0").as_deref(), Some("sha256:hijacked"));
}

#[test]
fn aliases_move_only_to_a_higher_stable_version() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let repository = registry.repository();
    let tags = ["{version}", "latest"];
    let alias = |steps: Vec<ArtifactStep>| steps.last().unwrap().step.clone();

    registry
        .publisher(images.load(), &tags)
        .publish(&release("1.1.0"))
        .unwrap();
    let latest = registry.tag("latest").unwrap();

    // An older maintenance release leaves `latest` alone.
    let older = registry.publisher(images.load(), &tags);
    let steps = older.publish(&release("1.0.1")).unwrap();
    assert_eq!(alias(steps), AssetStep::Kept("1.1.0 is higher".into()));
    assert_eq!(registry.tag("latest"), Some(latest.clone()));
    assert_eq!(
        older.state(&release("1.0.1")).unwrap(),
        PublisherState::Published
    );

    // A pre-release never moves it.
    let steps = registry
        .publisher(images.load(), &tags)
        .publish(&release("1.2.0-rc.1"))
        .unwrap();
    assert_eq!(alias(steps), AssetStep::Kept("pre-release".into()));
    assert_eq!(registry.tag("latest"), Some(latest.clone()));

    // A higher stable release moves it.
    let newer = registry.publisher(images.load(), &tags);
    assert_eq!(
        newer.state(&release("1.2.0")).unwrap(),
        PublisherState::Missing
    );
    let steps = newer.publish(&release("1.2.0")).unwrap();
    assert_eq!(
        steps.last().unwrap(),
        &step(
            ArtifactKind::Alias,
            &format!("{repository}:latest"),
            AssetStep::Uploaded
        )
    );
    assert_eq!(registry.tag("latest"), registry.tag("1.2.0"));

    // An alias pushed by something else, without a version, is not taken over.
    registry.set_tag("stable", &latest);
    {
        let mut state = registry.state.lock().unwrap();
        let foreign = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[]}"#.to_vec();
        let digest = digest(&foreign);
        state
            .manifests
            .insert(digest.clone(), (INDEX_MEDIA_TYPE.into(), foreign));
        state.tags.insert("stable".into(), digest);
    }
    let steps = registry
        .publisher(images.load(), &["{version}", "stable"])
        .publish(&release("1.3.0"))
        .unwrap();
    let AssetStep::Kept(reason) = alias(steps) else {
        panic!("expected the foreign alias to be kept");
    };
    assert!(reason.contains("without a version"), "{reason}");
}

#[test]
fn a_public_repository_is_read_anonymously_with_pull_access_only() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    registry
        .publisher(images.load(), &["{version}"])
        .publish(&release("1.0.0"))
        .unwrap();
    let pushed = registry.tag("1.0.0").unwrap();
    let mut state = registry.state.lock().unwrap();
    state.public = true;
    state.token_requests.clear();
    drop(state);

    let client = registry.anonymous_client();
    let (digest, _) = client.manifest("1.0.0").unwrap().unwrap();
    assert_eq!(digest, pushed);
    assert_eq!(client.manifest("2.0.0").unwrap(), None);

    let requests = registry.state.lock().unwrap().token_requests.clone();
    assert_eq!(
        requests,
        [(
            "service=mock&scope=repository%3Ateam%2Fapp%3Apull".to_owned(),
            None
        )]
    );
}

#[test]
fn a_private_repository_rejects_an_anonymous_read() {
    let registry = MockRegistry::start();
    let error = registry
        .anonymous_client()
        .manifest("1.0.0")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("does not seem to be publicly readable"),
        "{error}"
    );
    assert!(error.contains("public = true"), "{error}");
}

#[test]
fn credentials_are_checked_and_never_printed() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let publisher = registry.publisher_with(images.load(), &["{version}"], "wrong-password");
    let error = publisher.state(&release("1.0.0")).unwrap_err().to_string();
    assert!(error.contains("rejected the credentials"), "{error}");
    assert!(!error.contains("wrong-password"));
    assert!(!format!("{publisher:?}").contains("wrong-password"));

    let missing = presswerk::oci::Credentials::from_environment(
        "PRESSWERK_TEST_UNSET_USERNAME",
        "PRESSWERK_TEST_UNSET_PASSWORD",
    )
    .unwrap_err()
    .to_string();
    assert!(
        missing.contains("PRESSWERK_TEST_UNSET_USERNAME"),
        "{missing}"
    );
}

#[test]
fn a_failed_alias_is_a_warning_that_a_rerun_repairs() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let repository = registry.repository();
    let tags = ["{version}", "latest"];
    let release = release("1.0.0");
    registry.fault(Fault::RejectTag("latest"));

    let publisher = registry.publisher(images.load(), &tags);
    let steps = publisher.publish(&release).unwrap();
    let alias = steps.last().unwrap();
    assert_eq!(alias.name, format!("{repository}:latest"));
    let AssetStep::Warning(reason) = &alias.step else {
        panic!("expected a warning, got {alias:?}");
    };
    assert!(reason.contains("HTTP 500"), "{reason}");
    assert_eq!(
        steps[2].step,
        AssetStep::Uploaded,
        "the version tag is pushed"
    );
    assert!(registry.tag("latest").is_none());
    assert_eq!(
        publisher.state(&release).unwrap(),
        PublisherState::AliasesPending
    );

    let rerun = registry.publisher(images.load(), &tags);
    let steps = rerun.publish(&release).unwrap();
    assert_eq!(steps[2].step, AssetStep::AlreadyUploaded);
    assert_eq!(steps.last().unwrap().step, AssetStep::Uploaded);
    assert_eq!(registry.tag("latest"), registry.tag("1.0.0"));
    assert_eq!(rerun.state(&release).unwrap(), PublisherState::Published);
}

#[test]
fn an_unreadable_alias_is_a_warning_while_the_release_completes() {
    let registry = MockRegistry::start();
    let images = Images::new("hello");
    let release = release("1.0.0");
    registry.fault(Fault::UnreadableTag("latest"));

    let publisher = registry.publisher(images.load(), &["{version}", "latest"]);
    assert_eq!(publisher.state(&release).unwrap(), PublisherState::Missing);
    let steps = publisher.publish(&release).unwrap();
    assert_eq!(steps[2].step, AssetStep::Uploaded);
    let AssetStep::Warning(reason) = &steps.last().unwrap().step else {
        panic!("expected a warning");
    };
    assert!(reason.contains("cannot tell"), "{reason}");
    assert!(
        registry.tag("latest").is_none(),
        "an unreadable alias is not overwritten"
    );
    assert_eq!(
        publisher.state(&release).unwrap(),
        PublisherState::AliasesPending
    );
}
