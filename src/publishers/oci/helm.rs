//! The OCI Helm publisher: pushes a packaged chart as one OCI manifest under its release version.

use super::{LABELS, Registry, TAG};
use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::oci::{
    Blob, BlobData, Descriptor, MANIFEST_MEDIA_TYPE, Manifest, ORIGIN_ANNOTATION, RegistryClient,
    VERSION_ANNOTATION, sha256_digest, sha256_digest_reader,
};
use crate::publishers::shared::secrets::Credential;
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, shared,
};
use flate2::read::GzDecoder;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.cncf.helm.config.v1+json";
pub const CHART_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.helm.chart.content.v1.tar+gzip";

const CHART_METADATA_LIMIT: u64 = 1 << 20;

fn chart_error(path: &Path, action: &str, error: impl std::fmt::Display) -> PublicationError {
    PublicationError::new(format!(
        "could not {action} Helm chart {}: {error}",
        path.display()
    ))
}

/// Helm replaces `+` with `_` because OCI tags cannot carry SemVer build metadata.
#[must_use]
pub fn tag(version: &Version) -> String {
    version.to_string().replace('+', "_")
}

pub fn validate_tag(version: &Version) -> Result<String, String> {
    let tag = tag(version);
    (tag.len() <= 128).then_some(tag).ok_or_else(|| {
        format!("Helm chart version {version} renders an OCI tag longer than 128 characters")
    })
}

/// `publishers.oci-helm.<name>`: a packaged Helm chart pushed to an OCI registry under its
/// release version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciHelmDeclaration {
    /// Full registry repository path, ending in the chart name.
    pub repository: String,
    /// Chart archive written by `helm package`, as a path in the Nix store.
    pub chart: String,
    /// Whether anyone may pull the chart; the previous-release check then reads anonymously.
    #[serde(default)]
    pub public: bool,
    pub username: Credential,
    pub password: Credential,
}

impl OciHelmDeclaration {
    fn registry(&self) -> Registry<'_> {
        Registry {
            repository: &self.repository,
            public: self.public,
            username: &self.username,
            password: &self.password,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !self.chart.starts_with('/') {
            return Err(format!(
                "chart must be an absolute store path, not {:?}",
                self.chart
            ));
        }
        self.registry().validate()
    }
}

/// The `oci-helm` publisher type.
pub struct OciHelm {
    declaration: OciHelmDeclaration,
    /// Built on first use, once the chart is fetched and the credentials are known.
    runner: OnceLock<OciHelmPublisher>,
}

impl OciHelm {
    #[must_use]
    pub fn declaration(&self) -> &OciHelmDeclaration {
        &self.declaration
    }

    fn runner(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<&OciHelmPublisher, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let declaration = &self.declaration;
        let runner = OciHelmPublisher::new(
            declaration
                .registry()
                .writing_client(&ctx.session.secrets)?,
            &declaration.repository,
            &declaration.chart,
            &release.version,
        )?;
        Ok(self.runner.get_or_init(|| runner))
    }
}

impl PublisherType for OciHelm {
    const NAME: &'static str = "oci-helm";

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: OciHelmDeclaration = shared::decode(declaration)?;
        declaration.validate().map_err(DeclarationError::Invalid)?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

/// Whether the release tag of `release` points to its chart from `chart`, as `manifest` reads
/// the manifest a tag points to. A tag that cannot be read, such as when the registry denies
/// access or is unreachable, makes the release `Unknown`.
pub fn completeness_with(
    repository: &str,
    chart: &str,
    release: &Release,
    manifest: impl FnOnce(&str) -> Result<Option<Vec<u8>>, PublicationError>,
) -> Result<Completeness, PublicationError> {
    let version = &release.version;
    let tag = tag(version);
    let read = match manifest(&tag) {
        Ok(read) => read,
        Err(error) => return Ok(Completeness::Unknown(error.to_string())),
    };
    let found = match read {
        None => "the tag is missing",
        Some(manifest) if is_release_manifest(&manifest, version, chart) => {
            return Ok(Completeness::Complete);
        }
        Some(_) => "the tag does not point to this release's Helm chart",
    };
    Ok(Completeness::Incomplete {
        target: format!("OCI Helm release {repository}:{tag}"),
        reason: format!("{found}, expected version {version} from {chart}"),
    })
}

fn describe(repository: &str, version: &Version) -> Result<Description, String> {
    Ok(Description {
        target: repository.into(),
        items: vec![(
            "tag".into(),
            format!("{repository}:{}", validate_tag(version)?),
        )],
    })
}

impl Publisher for OciHelm {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.declaration.repository, version)
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        vec![("chart".into(), self.declaration.chart.clone())]
    }

    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        let declaration = &self.declaration;
        let registry = declaration
            .registry()
            .reading_client(&ctx.session.secrets)?;
        completeness_with(
            &declaration.repository,
            &declaration.chart,
            release,
            |tag| Ok(registry.manifest(tag)?.map(|(_, manifest)| manifest)),
        )
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

#[derive(Debug)]
struct ChartPackage {
    path: PathBuf,
    origin: String,
    config: Vec<u8>,
    config_descriptor: Descriptor,
    chart_descriptor: Descriptor,
}

impl ChartPackage {
    fn open(
        path: &str,
        repository: &str,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        let archive = PathBuf::from(path);
        let chart_size = archive
            .metadata()
            .map_err(|error| chart_error(&archive, "inspect", error))?
            .len();
        let chart_digest = sha256_digest_reader(
            File::open(&archive).map_err(|error| chart_error(&archive, "open", error))?,
        )
        .map_err(|error| chart_error(&archive, "read", error))?;
        let (root, metadata) = read_chart_metadata(&archive)?;
        let mut config: Value = serde_yaml_ng::from_slice(&metadata).map_err(|error| {
            PublicationError::new(format!("could not parse Chart.yaml in {path:?}: {error}"))
        })?;
        let object = config.as_object_mut().ok_or_else(|| {
            PublicationError::new(format!("Chart.yaml in {path:?} must be a YAML mapping"))
        })?;
        let field = |name: &str| {
            object
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    PublicationError::new(format!(
                        "Chart.yaml in {path:?} needs a non-empty string {name:?}"
                    ))
                })
        };
        let name = field("name")?.to_owned();
        let version_text = field("version")?.to_owned();
        if root != name {
            return Err(PublicationError::new(format!(
                "Helm chart {path:?} has root directory {root:?}, expected chart name {name:?}"
            )));
        }
        let version = Version::parse(&version_text).map_err(|error| {
            PublicationError::new(format!(
                "chart {name:?} in {path:?} has invalid version {version_text:?}: {error}"
            ))
        })?;
        if version != *release_version {
            return Err(PublicationError::new(format!(
                "Helm chart {path:?} has version {version}, expected release version {release_version}"
            )));
        }
        if repository.rsplit('/').next().unwrap_or_default() != name {
            return Err(PublicationError::new(format!(
                "Helm repository {repository:?} must end in chart name {name:?}"
            )));
        }
        if let Some(app_version @ (Value::Number(_) | Value::Bool(_))) =
            object.get_mut("appVersion")
        {
            *app_version = Value::String(app_version.to_string());
        }
        let config = serde_json::to_vec(&config).expect("YAML converted to JSON serializes");
        Ok(Self {
            path: archive,
            origin: path.into(),
            config_descriptor: Descriptor::new(
                CONFIG_MEDIA_TYPE,
                &sha256_digest(&config),
                config.len() as u64,
            ),
            chart_descriptor: Descriptor::new(CHART_LAYER_MEDIA_TYPE, &chart_digest, chart_size),
            config,
        })
    }

    fn manifest(&self, version: &Version) -> (Vec<u8>, String) {
        let manifest = Manifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE.into(),
            config: self.config_descriptor.clone(),
            layers: vec![self.chart_descriptor.clone()],
            annotations: BTreeMap::from([
                (VERSION_ANNOTATION.into(), version.to_string()),
                (ORIGIN_ANNOTATION.into(), self.origin.clone()),
            ]),
        };
        let bytes = serde_json::to_vec(&manifest).expect("Helm manifest serializes");
        let digest = sha256_digest(&bytes);
        (bytes, digest)
    }
}

fn read_chart_metadata(path: &Path) -> Result<(String, Vec<u8>), PublicationError> {
    let file = File::open(path).map_err(|error| chart_error(path, "open", error))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut found = None;
    for entry in archive
        .entries()
        .map_err(|error| chart_error(path, "read", error))?
    {
        let mut entry = entry.map_err(|error| chart_error(path, "read", error))?;
        let entry_path = entry
            .path()
            .map_err(|error| chart_error(path, "read a path in", error))?;
        let components: Vec<_> = entry_path.components().collect();
        let [Component::Normal(root), Component::Normal(name)] = components.as_slice() else {
            continue;
        };
        if name.to_str() != Some("Chart.yaml") {
            continue;
        }
        let root = root
            .to_str()
            .ok_or_else(|| {
                PublicationError::new(format!(
                    "Helm chart {:?} has a non-UTF-8 root directory",
                    path.display()
                ))
            })?
            .to_owned();
        if found.is_some() {
            return Err(PublicationError::new(format!(
                "Helm chart {:?} contains more than one top-level Chart.yaml",
                path.display()
            )));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(CHART_METADATA_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                PublicationError::new(format!(
                    "could not read Chart.yaml in {:?}: {error}",
                    path.display()
                ))
            })?;
        if bytes.len() as u64 > CHART_METADATA_LIMIT {
            return Err(PublicationError::new(format!(
                "Chart.yaml in {:?} is larger than 1 MiB",
                path.display()
            )));
        }
        found = Some((root, bytes));
    }
    found.ok_or_else(|| {
        PublicationError::new(format!(
            "Helm chart {:?} has no top-level Chart.yaml",
            path.display()
        ))
    })
}

fn identity(manifest: &[u8]) -> Option<(Version, String)> {
    let manifest: Manifest = serde_json::from_slice(manifest).ok()?;
    if manifest.schema_version != 2
        || manifest.media_type != MANIFEST_MEDIA_TYPE
        || manifest.config.media_type != CONFIG_MEDIA_TYPE
        || manifest.layers.len() != 1
        || manifest.layers[0].media_type != CHART_LAYER_MEDIA_TYPE
    {
        return None;
    }
    Some((
        Version::parse(manifest.annotations.get(VERSION_ANNOTATION)?).ok()?,
        manifest.annotations.get(ORIGIN_ANNOTATION)?.clone(),
    ))
}

#[must_use]
pub(crate) fn is_release_manifest(manifest: &[u8], version: &Version, origin: &str) -> bool {
    identity(manifest).is_some_and(|found| found == (version.clone(), origin.into()))
}

pub struct OciHelmPublisher {
    registry: RegistryClient,
    repository: String,
    chart: ChartPackage,
}

impl OciHelmPublisher {
    pub fn new(
        registry: RegistryClient,
        repository: &str,
        chart_path: &str,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            registry,
            repository: repository.into(),
            chart: ChartPackage::open(chart_path, repository, release_version)?,
        })
    }

    fn reference(&self, release: &Release) -> String {
        format!("{}:{}", self.repository, tag(&release.version))
    }

    fn tag_step(&self, release: &Release, step: AssetStep) -> ArtifactSteps {
        vec![ArtifactStep {
            kind: TAG,
            name: self.reference(release),
            step,
        }]
    }

    fn is_missing(&self, release: &Release) -> Result<bool, PublicationError> {
        match self.checked_state(release)? {
            PublisherState::Missing => Ok(true),
            PublisherState::Published => Ok(false),
            PublisherState::Conflicting(message) => Err(PublicationError::new(message)),
            _ => unreachable!("OCI Helm publisher state"),
        }
    }
}

/// An OCI Helm publisher with its own registry client, such as one for a test registry.
impl Publisher for OciHelmPublisher {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.repository, version)
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        completeness_with(&self.repository, &self.chart.origin, release, |tag| {
            Ok(self.registry.manifest(tag)?.map(|(_, manifest)| manifest))
        })
    }

    fn state(&self, release: &Release, _ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        let steps = self.push_chart(release)?;
        steps.iter().for_each(|step| ctx.progress(step));
        Ok(steps)
    }

    fn complete_steps(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        Ok(self.tag_step(release, AssetStep::AlreadyUploaded))
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

impl OciHelmPublisher {
    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        let tag = validate_tag(&release.version).map_err(PublicationError::new)?;
        Ok(match self.registry.manifest(&tag)? {
            None => PublisherState::Missing,
            Some((_, manifest))
                if is_release_manifest(&manifest, &release.version, &self.chart.origin) =>
            {
                PublisherState::Published
            }
            Some((digest, manifest)) => {
                let description = identity(&manifest).map_or_else(
                    || "an unrecognized manifest".to_owned(),
                    |(version, origin)| format!("version {version} from {origin}"),
                );
                PublisherState::Conflicting(format!(
                    "{} points to {digest}, {description}; expected version {} from {}",
                    self.reference(release),
                    release.version,
                    self.chart.origin
                ))
            }
        })
    }
}

impl OciHelmPublisher {
    fn push_chart(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        if !self.is_missing(release)? {
            return Ok(self.tag_step(release, AssetStep::AlreadyUploaded));
        }

        self.registry.push_blob(&Blob {
            digest: self.chart.config_descriptor.digest.clone(),
            size: self.chart.config_descriptor.size,
            data: BlobData::Bytes(&self.chart.config),
        })?;
        let chart = File::open(&self.chart.path).map_err(|error| {
            PublicationError::new(format!(
                "could not open Helm chart {:?}: {error}",
                self.chart.path.display()
            ))
        })?;
        self.registry.push_blob(&Blob {
            digest: self.chart.chart_descriptor.digest.clone(),
            size: self.chart.chart_descriptor.size,
            data: BlobData::File(&chart),
        })?;

        let (manifest, digest) = self.chart.manifest(&release.version);
        self.registry
            .push_manifest(MANIFEST_MEDIA_TYPE, &manifest, &digest)?;
        if self.is_missing(release)? {
            self.registry.put_manifest(
                &tag(&release.version),
                MANIFEST_MEDIA_TYPE,
                &manifest,
                &digest,
            )?;
        }
        if self.is_missing(release)? {
            return Err(PublicationError::new(format!(
                "{} was not published",
                self.reference(release)
            )));
        }
        Ok(self.tag_step(release, AssetStep::Uploaded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::{PublisherNode, RemoteState, TagPublisher, publish_ordered};
    use crate::publishers::Context;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn chart(directory: &Path, name: &str, version: &str) -> PathBuf {
        chart_with_root(directory, name, name, version)
    }

    fn chart_with_root(directory: &Path, root: &str, name: &str, version: &str) -> PathBuf {
        let path = directory.join(format!("{name}-{version}.tgz"));
        let encoder = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let metadata = format!(
            "apiVersion: v2\nname: {name}\ndescription: Test chart\ntype: application\nversion: {version}\nappVersion: {version}\nannotations:\n  example: value\n"
        );
        for (entry, contents) in [
            (format!("{root}/Chart.yaml"), metadata.as_bytes()),
            (
                format!("{root}/templates/deployment.yaml"),
                b"kind: Deployment\n".as_slice(),
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(1);
            archive.append_data(&mut header, entry, contents).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        path
    }

    #[derive(Default)]
    struct RegistryState {
        blobs: HashMap<String, Vec<u8>>,
        manifests: HashMap<String, Vec<u8>>,
        references: HashMap<String, String>,
        writes: Vec<String>,
        inspection_fails: bool,
    }

    struct MockRegistry {
        repository: String,
        state: Arc<Mutex<RegistryState>>,
    }

    impl MockRegistry {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let repository = format!("{}/team/project", listener.local_addr().unwrap());
            let state = Arc::new(Mutex::new(RegistryState::default()));
            let shared = state.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    let state = shared.clone();
                    thread::spawn(move || handle(stream, &state));
                }
            });
            Self { repository, state }
        }

        fn publisher(&self, chart: &Path, version: &Version) -> OciHelmPublisher {
            OciHelmPublisher::new(
                RegistryClient::new(&self.repository, None).unwrap(),
                &self.repository,
                &chart.to_string_lossy(),
                version,
            )
            .unwrap()
        }
    }

    fn handle(mut stream: TcpStream, state: &Mutex<RegistryState>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        let mut parts = request.split_whitespace();
        let method = parts.next().unwrap();
        let target = parts.next().unwrap();
        let mut length = 0;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header == "\r\n" {
                break;
            }
            if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let path = target.split('?').next().unwrap();
        let mut state = state.lock().unwrap();
        let fail = state.inspection_fails
            && matches!(method, "GET" | "HEAD")
            && path.contains("/manifests/");
        if fail {
            return response(&mut stream, 500, &[], &[]);
        }
        if method == "HEAD"
            && let Some(digest) = path.strip_prefix("/v2/team/project/blobs/")
        {
            return response(
                &mut stream,
                if state.blobs.contains_key(digest) {
                    200
                } else {
                    404
                },
                &[],
                &[],
            );
        }
        if let Some(reference) = path.strip_prefix("/v2/team/project/manifests/") {
            if method == "PUT" {
                let digest = sha256_digest(&body);
                state.manifests.insert(digest.clone(), body);
                state.references.insert(reference.into(), digest.clone());
                state.writes.push(format!("manifest {reference}"));
                return response(&mut stream, 201, &[("Docker-Content-Digest", &digest)], &[]);
            }
            let digest = state.references.get(reference).cloned().or_else(|| {
                state
                    .manifests
                    .contains_key(reference)
                    .then(|| reference.into())
            });
            let Some(digest) = digest else {
                return response(&mut stream, 404, &[], &[]);
            };
            if method == "HEAD" {
                return response(&mut stream, 200, &[("Docker-Content-Digest", &digest)], &[]);
            }
            let manifest = state.manifests[&digest].clone();
            return response(&mut stream, 200, &[], &manifest);
        }
        if method == "POST" && path == "/v2/team/project/blobs/uploads/" {
            return response(&mut stream, 202, &[("Location", "/upload")], &[]);
        }
        if method == "PUT" && path == "/upload" {
            let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let digest = url
                .query_pairs()
                .find(|(key, _)| key == "digest")
                .unwrap()
                .1
                .into_owned();
            assert_eq!(sha256_digest(&body), digest);
            state.blobs.insert(digest.clone(), body);
            state.writes.push(format!("blob {digest}"));
            return response(&mut stream, 201, &[], &[]);
        }
        response(&mut stream, 404, &[], &[]);
    }

    fn response(stream: &mut TcpStream, status: u16, headers: &[(&str, &str)], body: &[u8]) {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => unreachable!(),
        };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        stream.write_all(body).unwrap();
    }

    fn release(version: &str) -> Release {
        Release {
            commit: "0123456789012345678901234567890123456789".into(),
            version: Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            notes: String::new(),
        }
    }

    #[test]
    fn pushes_one_oci_helm_manifest_and_a_retry_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let registry = MockRegistry::start();
        let release = release("1.2.3");
        let publisher = registry.publisher(&chart, &release.version);

        assert_eq!(
            publisher.state(&release, &Context::detached()).unwrap(),
            PublisherState::Missing
        );
        assert_eq!(
            publisher.publish(&release, &Context::detached()).unwrap(),
            [ArtifactStep {
                kind: TAG,
                name: format!("{}:1.2.3", registry.repository),
                step: AssetStep::Uploaded,
            }]
        );
        assert_eq!(
            publisher.state(&release, &Context::detached()).unwrap(),
            PublisherState::Published
        );

        let writes = registry.state.lock().unwrap().writes.clone();
        let manifest = {
            let state = registry.state.lock().unwrap();
            let digest = &state.references["1.2.3"];
            state.manifests[digest].clone()
        };
        let manifest: Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(manifest["mediaType"], MANIFEST_MEDIA_TYPE);
        assert_eq!(manifest["config"]["mediaType"], CONFIG_MEDIA_TYPE);
        assert_eq!(manifest["layers"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["layers"][0]["mediaType"], CHART_LAYER_MEDIA_TYPE);
        assert_eq!(
            manifest["annotations"],
            json!({
                VERSION_ANNOTATION: "1.2.3",
                ORIGIN_ANNOTATION: chart.to_string_lossy(),
            })
        );
        let config_digest = manifest["config"]["digest"].as_str().unwrap();
        let config: Value =
            serde_json::from_slice(&registry.state.lock().unwrap().blobs[config_digest]).unwrap();
        assert_eq!(config["name"], "project");
        assert_eq!(config["version"], "1.2.3");
        assert_eq!(config["appVersion"], "1.2.3");

        assert_eq!(
            publisher.publish(&release, &Context::detached()).unwrap(),
            [ArtifactStep {
                kind: TAG,
                name: format!("{}:1.2.3", registry.repository),
                step: AssetStep::AlreadyUploaded,
            }]
        );
        assert_eq!(registry.state.lock().unwrap().writes, writes);
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
    fn another_store_path_conflicts_before_the_git_tag_or_registry_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let first = chart(directory.path(), "project", "1.2.3");
        let other_directory = tempfile::tempdir().unwrap();
        let other = chart(other_directory.path(), "project", "1.2.3");
        let registry = MockRegistry::start();
        let release = release("1.2.3");
        registry
            .publisher(&first, &release.version)
            .publish(&release, &Context::detached())
            .unwrap();
        let writes_before = registry.state.lock().unwrap().writes.clone();
        let conflicting = registry.publisher(&other, &release.version);
        let tag_writes = Rc::new(Cell::new(0));

        let error = publish_ordered(
            &FakeTagPublisher {
                writes: tag_writes.clone(),
            },
            &release,
            &[PublisherNode::new(
                "test",
                Box::new(conflicting),
                Vec::new(),
            )],
            false,
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(&other.to_string_lossy().to_string()),
            "{error}"
        );
        assert_eq!(tag_writes.get(), 0);
        assert_eq!(registry.state.lock().unwrap().writes, writes_before);
    }

    #[test]
    fn an_unknown_remote_state_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let registry = MockRegistry::start();
        registry.state.lock().unwrap().inspection_fails = true;
        let release = release("1.2.3");
        let publisher = registry.publisher(&chart, &release.version);

        let error = publisher
            .state(&release, &Context::detached())
            .unwrap_err()
            .to_string();

        assert!(error.contains("HTTP 500"), "{error}");
        assert!(registry.state.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn chart_name_and_version_are_checked_before_publication() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let path = chart.to_string_lossy();
        let wrong_version = ChartPackage::open(
            &path,
            "ghcr.io/owner/charts/project",
            &Version::new(1, 2, 4),
        )
        .unwrap_err()
        .to_string();
        assert!(wrong_version.contains("expected release version 1.2.4"));

        let wrong_name =
            ChartPackage::open(&path, "ghcr.io/owner/charts/other", &Version::new(1, 2, 3))
                .unwrap_err()
                .to_string();
        assert!(wrong_name.contains("must end in chart name \"project\""));

        let wrong_root = chart_with_root(directory.path(), "other", "project", "1.2.3");
        let wrong_root = ChartPackage::open(
            &wrong_root.to_string_lossy(),
            "ghcr.io/owner/charts/project",
            &Version::new(1, 2, 3),
        )
        .unwrap_err()
        .to_string();
        assert!(
            wrong_root.contains("root directory \"other\""),
            "{wrong_root}"
        );
    }

    #[test]
    fn an_unreadable_chart_tag_is_unknown_not_incomplete() {
        let completeness = completeness_with(
            "ghcr.io/owner/charts/project",
            "/nix/store/abc-project.tgz",
            &release("1.2.3"),
            |_| Err(PublicationError::new("could not reach the registry")),
        )
        .unwrap();
        assert_eq!(
            completeness,
            Completeness::Unknown("could not reach the registry".into())
        );
    }

    #[test]
    fn a_version_whose_tag_is_too_long_cannot_be_described() {
        let helm = OciHelm::from_declaration(&json!({
            "repository": "ghcr.io/owner/charts/project",
            "chart": "/nix/store/abc-project.tgz",
            "username": { "value": "user" },
            "password": { "secret": { "env": "REGISTRY_TOKEN" } }
        }))
        .unwrap();
        let version = Version::parse(&format!("1.2.3-{}", "a".repeat(123))).unwrap();

        let error = helm.describe(&version, "v1").unwrap_err();
        assert!(error.contains("longer than 128 characters"), "{error}");
        assert!(helm.describe(&Version::new(1, 2, 3), "v1.2.3").is_ok());
    }

    #[test]
    fn build_metadata_uses_helms_underscore_tag() {
        assert_eq!(
            validate_tag(&Version::parse("1.2.3+build.4").unwrap()).unwrap(),
            "1.2.3_build.4"
        );
        assert!(
            validate_tag(&Version::parse(&format!("1.2.3+{}", "a".repeat(123))).unwrap()).is_err()
        );
    }
}
