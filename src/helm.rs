//! The Helm publisher: pushes a packaged chart as one OCI manifest under its release version.

use crate::oci::{
    Blob, BlobData, Credentials, MANIFEST_MEDIA_TYPE, ORIGIN_ANNOTATION, RegistryClient,
    VERSION_ANNOTATION,
};
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, Publisher,
    PublisherState, Release,
};
use flate2::read::GzDecoder;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.cncf.helm.config.v1+json";
pub const CHART_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.helm.chart.content.v1.tar+gzip";

const CHART_METADATA_LIMIT: u64 = 1 << 20;

/// Helm replaces `+` with `_` because OCI tags cannot carry SemVer build metadata.
#[must_use]
pub fn tag(version: &Version) -> String {
    version.to_string().replace('+', "_")
}

pub fn validate_tag(version: &Version) -> Result<String, String> {
    let tag = tag(version);
    if tag.len() > 128 {
        return Err(format!(
            "Helm chart version {version} renders an OCI tag longer than 128 characters"
        ));
    }
    Ok(tag)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: &'static str,
    digest: String,
    size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    media_type: &'static str,
    config: Descriptor,
    layers: [Descriptor; 1],
    annotations: BTreeMap<String, String>,
}

#[derive(Debug)]
struct ChartPackage {
    path: PathBuf,
    origin: String,
    name: String,
    version: Version,
    config: Vec<u8>,
    config_digest: String,
    chart_digest: String,
    chart_size: u64,
}

impl ChartPackage {
    fn open(path: &str) -> Result<Self, PublicationError> {
        let path_buf = PathBuf::from(path);
        let chart_size = path_buf
            .metadata()
            .map_err(|error| {
                PublicationError::new(format!("could not inspect Helm chart {path:?}: {error}"))
            })?
            .len();
        let chart_digest = digest_reader(File::open(&path_buf).map_err(|error| {
            PublicationError::new(format!("could not open Helm chart {path:?}: {error}"))
        })?)?;
        let metadata = read_chart_metadata(&path_buf)?;
        let documents = YamlLoader::load_from_str(&metadata).map_err(|error| {
            PublicationError::new(format!("could not parse Chart.yaml in {path:?}: {error}"))
        })?;
        let [document] = documents.as_slice() else {
            return Err(PublicationError::new(format!(
                "Chart.yaml in {path:?} must contain exactly one YAML document"
            )));
        };
        let mapping = document.as_hash().ok_or_else(|| {
            PublicationError::new(format!("Chart.yaml in {path:?} must be a YAML mapping"))
        })?;
        let field = |name: &str| {
            mapping
                .get(&Yaml::String(name.into()))
                .and_then(Yaml::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    PublicationError::new(format!(
                        "Chart.yaml in {path:?} needs a non-empty string {name:?}"
                    ))
                })
        };
        let name = field("name")?.to_owned();
        let version_text = field("version")?;
        let version = Version::parse(version_text).map_err(|error| {
            PublicationError::new(format!(
                "chart {name:?} in {path:?} has invalid version {version_text:?}: {error}"
            ))
        })?;
        let mut config_value = yaml_json(document).map_err(|reason| {
            PublicationError::new(format!(
                "could not convert Chart.yaml in {path:?}: {reason}"
            ))
        })?;
        let object = config_value
            .as_object_mut()
            .expect("Chart.yaml mapping converted to a JSON object");
        object.insert("name".into(), Value::String(name.clone()));
        object.insert("version".into(), Value::String(version_text.into()));
        if let Some(app_version) = mapping.get(&Yaml::String("appVersion".into()))
            && let Some(app_version) = yaml_scalar_string(app_version)
        {
            object.insert("appVersion".into(), Value::String(app_version));
        }
        let config = serde_json::to_vec(&config_value).expect("YAML converted to JSON serializes");
        let config_digest = digest(&config);
        Ok(Self {
            path: path_buf,
            origin: path.into(),
            name,
            version,
            config,
            config_digest,
            chart_digest,
            chart_size,
        })
    }

    fn manifest(&self, version: &Version) -> (Vec<u8>, String) {
        let manifest = Manifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE,
            config: Descriptor {
                media_type: CONFIG_MEDIA_TYPE,
                digest: self.config_digest.clone(),
                size: self.config.len() as u64,
            },
            layers: [Descriptor {
                media_type: CHART_LAYER_MEDIA_TYPE,
                digest: self.chart_digest.clone(),
                size: self.chart_size,
            }],
            annotations: BTreeMap::from([
                (VERSION_ANNOTATION.into(), version.to_string()),
                (ORIGIN_ANNOTATION.into(), self.origin.clone()),
            ]),
        };
        let bytes = serde_json::to_vec(&manifest).expect("Helm manifest serializes");
        let digest = digest(&bytes);
        (bytes, digest)
    }
}

fn read_chart_metadata(path: &Path) -> Result<String, PublicationError> {
    let file = File::open(path).map_err(|error| {
        PublicationError::new(format!(
            "could not open Helm chart {:?}: {error}",
            path.display()
        ))
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut found = None;
    let entries = archive.entries().map_err(|error| {
        PublicationError::new(format!(
            "could not read Helm chart {:?}: {error}",
            path.display()
        ))
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| {
            PublicationError::new(format!(
                "could not read Helm chart {:?}: {error}",
                path.display()
            ))
        })?;
        let entry_path = entry.path().map_err(|error| {
            PublicationError::new(format!(
                "could not read a path in Helm chart {:?}: {error}",
                path.display()
            ))
        })?;
        let components: Vec<_> = entry_path.components().collect();
        if !matches!(
            components.as_slice(),
            [Component::Normal(_), Component::Normal(name)] if *name == "Chart.yaml"
        ) {
            continue;
        }
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
        found = Some(String::from_utf8(bytes).map_err(|error| {
            PublicationError::new(format!(
                "Chart.yaml in {:?} is not UTF-8: {error}",
                path.display()
            ))
        })?);
    }
    found.ok_or_else(|| {
        PublicationError::new(format!(
            "Helm chart {:?} has no top-level Chart.yaml",
            path.display()
        ))
    })
}

fn yaml_scalar_string(value: &Yaml) -> Option<String> {
    match value {
        Yaml::String(value) | Yaml::Real(value) => Some(value.clone()),
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn yaml_json(value: &Yaml) -> Result<Value, String> {
    Ok(match value {
        Yaml::Real(value) => Value::Number(
            value
                .parse::<serde_json::Number>()
                .map_err(|_| format!("number {value:?} is not valid JSON"))?,
        ),
        Yaml::Integer(value) => Value::Number((*value).into()),
        Yaml::String(value) => Value::String(value.clone()),
        Yaml::Boolean(value) => Value::Bool(*value),
        Yaml::Array(values) => Value::Array(
            values
                .iter()
                .map(yaml_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Yaml::Hash(values) => {
            let mut object = Map::new();
            for (key, value) in values {
                let Yaml::String(key) = key else {
                    return Err("mapping keys must be strings".into());
                };
                object.insert(key.clone(), yaml_json(value)?);
            }
            Value::Object(object)
        }
        Yaml::Null => Value::Null,
        Yaml::Alias(_) | Yaml::BadValue => {
            return Err("aliases and invalid values are not supported".into());
        }
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn digest_reader(mut reader: impl Read) -> Result<String, PublicationError> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            PublicationError::new(format!("could not read Helm chart archive: {error}"))
        })?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex(&hash.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

trait Registry {
    fn manifest(&self, reference: &str) -> Result<Option<(String, Vec<u8>)>, PublicationError>;
    fn manifest_digest(&self, reference: &str) -> Result<Option<String>, PublicationError>;
    fn blob_exists(&self, digest: &str) -> Result<bool, PublicationError>;
    fn upload_blob(&self, blob: &Blob<'_>) -> Result<(), PublicationError>;
    fn put_manifest(
        &self,
        reference: &str,
        media_type: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<(), PublicationError>;
}

impl Registry for RegistryClient {
    fn manifest(&self, reference: &str) -> Result<Option<(String, Vec<u8>)>, PublicationError> {
        RegistryClient::manifest(self, reference)
    }

    fn manifest_digest(&self, reference: &str) -> Result<Option<String>, PublicationError> {
        RegistryClient::manifest_digest(self, reference)
    }

    fn blob_exists(&self, digest: &str) -> Result<bool, PublicationError> {
        RegistryClient::blob_exists(self, digest)
    }

    fn upload_blob(&self, blob: &Blob<'_>) -> Result<(), PublicationError> {
        RegistryClient::upload_blob(self, blob)
    }

    fn put_manifest(
        &self,
        reference: &str,
        media_type: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<(), PublicationError> {
        RegistryClient::put_manifest(self, reference, media_type, bytes, digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    version: Version,
    origin: String,
}

fn identity(manifest: &[u8]) -> Option<Identity> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RemoteDescriptor {
        media_type: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RemoteManifest {
        schema_version: u32,
        media_type: String,
        config: RemoteDescriptor,
        layers: Vec<RemoteDescriptor>,
        #[serde(default)]
        annotations: BTreeMap<String, String>,
    }
    let manifest: RemoteManifest = serde_json::from_slice(manifest).ok()?;
    if manifest.schema_version != 2
        || manifest.media_type != MANIFEST_MEDIA_TYPE
        || manifest.config.media_type != CONFIG_MEDIA_TYPE
        || manifest.layers.len() != 1
        || manifest.layers[0].media_type != CHART_LAYER_MEDIA_TYPE
    {
        return None;
    }
    Some(Identity {
        version: Version::parse(manifest.annotations.get(VERSION_ANNOTATION)?).ok()?,
        origin: manifest.annotations.get(ORIGIN_ANNOTATION)?.clone(),
    })
}

#[must_use]
pub(crate) fn is_release_manifest(manifest: &[u8], version: &Version, origin: &str) -> bool {
    identity(manifest).is_some_and(|found| found.version == *version && found.origin == origin)
}

enum TagState {
    Missing,
    Matching,
    Conflicting(String),
}

pub struct HelmPublisher {
    registry: Box<dyn Registry>,
    repository: String,
    chart: ChartPackage,
}

fn checked_chart(
    repository: &str,
    chart_path: &str,
    release_version: &Version,
) -> Result<ChartPackage, PublicationError> {
    let chart = ChartPackage::open(chart_path)?;
    if chart.version != *release_version {
        return Err(PublicationError::new(format!(
            "Helm chart {chart_path:?} has version {}, expected release version {release_version}",
            chart.version
        )));
    }
    let repository_name = repository.rsplit('/').next().unwrap_or_default();
    if repository_name != chart.name {
        return Err(PublicationError::new(format!(
            "Helm repository {repository:?} must end in chart name {:?}",
            chart.name
        )));
    }
    Ok(chart)
}

impl HelmPublisher {
    /// Build the publisher from its declaration; its chart must already be fetched.
    pub fn from_declaration(
        declaration: &crate::declaration::HelmPublisher,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        let chart = checked_chart(&declaration.repository, &declaration.chart, release_version)?;
        let credentials =
            Credentials::from_environment(&declaration.username, &declaration.password.secret)?;
        let registry = RegistryClient::new(&declaration.repository, Some(credentials))?;
        Ok(Self {
            registry: Box::new(registry),
            repository: declaration.repository.clone(),
            chart,
        })
    }

    fn reference(&self, release: &Release) -> String {
        format!("{}:{}", self.repository, tag(&release.version))
    }

    fn tag_state(&self, release: &Release) -> Result<TagState, PublicationError> {
        let tag = validate_tag(&release.version).map_err(PublicationError::new)?;
        Ok(match self.registry.manifest(&tag)? {
            None => TagState::Missing,
            Some((_, manifest))
                if is_release_manifest(&manifest, &release.version, &self.chart.origin) =>
            {
                TagState::Matching
            }
            Some((digest, manifest)) => {
                let description = identity(&manifest).map_or_else(
                    || "an unrecognized manifest".to_owned(),
                    |identity| format!("version {} from {}", identity.version, identity.origin),
                );
                TagState::Conflicting(format!(
                    "{} points to {digest}, {description}; expected version {} from {}",
                    self.reference(release),
                    release.version,
                    self.chart.origin
                ))
            }
        })
    }

    fn push_blob(
        &self,
        digest: &str,
        size: u64,
        data: BlobData<'_>,
    ) -> Result<(), PublicationError> {
        if !self.registry.blob_exists(digest)? {
            self.registry.upload_blob(&Blob {
                digest: digest.into(),
                size,
                data,
            })?;
        }
        Ok(())
    }

    fn tag_step(&self, release: &Release, step: AssetStep) -> ArtifactSteps {
        vec![ArtifactStep {
            kind: ArtifactKind::Tag,
            name: self.reference(release),
            step,
        }]
    }
}

impl Publisher for HelmPublisher {
    fn state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(match self.tag_state(release)? {
            TagState::Missing => PublisherState::Missing,
            TagState::Matching => PublisherState::Published,
            TagState::Conflicting(message) => PublisherState::Conflicting(message),
        })
    }

    fn publish(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        match self.tag_state(release)? {
            TagState::Matching => {
                return Ok(self.tag_step(release, AssetStep::AlreadyUploaded));
            }
            TagState::Conflicting(message) => return Err(PublicationError::new(message)),
            TagState::Missing => {}
        }

        self.push_blob(
            &self.chart.config_digest,
            self.chart.config.len() as u64,
            BlobData::Bytes(&self.chart.config),
        )?;
        let mut chart = File::open(&self.chart.path).map_err(|error| {
            PublicationError::new(format!(
                "could not open Helm chart {:?}: {error}",
                self.chart.path.display()
            ))
        })?;
        chart.rewind().map_err(|error| {
            PublicationError::new(format!(
                "could not read Helm chart {:?}: {error}",
                self.chart.path.display()
            ))
        })?;
        self.push_blob(
            &self.chart.chart_digest,
            self.chart.chart_size,
            BlobData::File(&chart),
        )?;

        let (manifest, manifest_digest) = self.chart.manifest(&release.version);
        if self.registry.manifest_digest(&manifest_digest)?.is_none() {
            self.registry.put_manifest(
                &manifest_digest,
                MANIFEST_MEDIA_TYPE,
                &manifest,
                &manifest_digest,
            )?;
        }

        match self.tag_state(release)? {
            TagState::Missing => self.registry.put_manifest(
                &tag(&release.version),
                MANIFEST_MEDIA_TYPE,
                &manifest,
                &manifest_digest,
            )?,
            TagState::Matching => {}
            TagState::Conflicting(message) => return Err(PublicationError::new(message)),
        }
        match self.tag_state(release)? {
            TagState::Matching => Ok(self.tag_step(release, AssetStep::Uploaded)),
            TagState::Missing => Err(PublicationError::new(format!(
                "{} was not published",
                self.reference(release)
            ))),
            TagState::Conflicting(message) => Err(PublicationError::new(message)),
        }
    }

    fn complete_steps(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        Ok(self.tag_step(release, AssetStep::AlreadyUploaded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::{RemoteState, TagPublisher, publish_ordered};
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    fn chart(directory: &Path, name: &str, version: &str) -> PathBuf {
        let path = directory.join(format!("{name}-{version}.tgz"));
        let encoder = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let metadata = format!(
            "apiVersion: v2\nname: {name}\ndescription: Test chart\ntype: application\nversion: {version}\nappVersion: {version}\nannotations:\n  example: value\n"
        );
        for (entry, contents) in [
            (format!("{name}/Chart.yaml"), metadata.as_bytes()),
            (
                format!("{name}/templates/deployment.yaml"),
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
    struct FakeState {
        blobs: HashMap<String, Vec<u8>>,
        manifests: HashMap<String, Vec<u8>>,
        references: HashMap<String, String>,
        writes: Vec<String>,
        inspection_fails: bool,
    }

    #[derive(Clone)]
    struct FakeRegistry(Rc<RefCell<FakeState>>);

    impl Registry for FakeRegistry {
        fn manifest(&self, reference: &str) -> Result<Option<(String, Vec<u8>)>, PublicationError> {
            let state = self.0.borrow();
            if state.inspection_fails {
                return Err(PublicationError::new(
                    "could not inspect manifest: registry unavailable",
                ));
            }
            let digest = state.references.get(reference).cloned().or_else(|| {
                state
                    .manifests
                    .contains_key(reference)
                    .then(|| reference.into())
            });
            Ok(digest.and_then(|digest| {
                state
                    .manifests
                    .get(&digest)
                    .cloned()
                    .map(|manifest| (digest, manifest))
            }))
        }

        fn manifest_digest(&self, reference: &str) -> Result<Option<String>, PublicationError> {
            Ok(self.manifest(reference)?.map(|(digest, _)| digest))
        }

        fn blob_exists(&self, digest: &str) -> Result<bool, PublicationError> {
            Ok(self.0.borrow().blobs.contains_key(digest))
        }

        fn upload_blob(&self, blob: &Blob<'_>) -> Result<(), PublicationError> {
            let bytes = match blob.data {
                BlobData::File(file) => {
                    let mut file = file;
                    file.rewind().unwrap();
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).unwrap();
                    bytes
                }
                BlobData::Bytes(bytes) => bytes.to_vec(),
            };
            assert_eq!(digest(&bytes), blob.digest);
            assert_eq!(bytes.len() as u64, blob.size);
            let mut state = self.0.borrow_mut();
            state.blobs.insert(blob.digest.clone(), bytes);
            state.writes.push(format!("blob {}", blob.digest));
            Ok(())
        }

        fn put_manifest(
            &self,
            reference: &str,
            media_type: &str,
            bytes: &[u8],
            expected_digest: &str,
        ) -> Result<(), PublicationError> {
            assert_eq!(media_type, MANIFEST_MEDIA_TYPE);
            assert_eq!(digest(bytes), expected_digest);
            let mut state = self.0.borrow_mut();
            state
                .manifests
                .insert(expected_digest.into(), bytes.to_vec());
            state
                .references
                .insert(reference.into(), expected_digest.into());
            state.writes.push(format!("manifest {reference}"));
            Ok(())
        }
    }

    fn make_publisher(
        state: Rc<RefCell<FakeState>>,
        repository: &str,
        path: &Path,
        version: &Version,
    ) -> HelmPublisher {
        let path = path.to_string_lossy();
        HelmPublisher {
            registry: Box::new(FakeRegistry(state)),
            repository: repository.into(),
            chart: checked_chart(repository, &path, version).unwrap(),
        }
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
    fn pushes_one_helm_manifest_and_a_retry_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let state = Rc::new(RefCell::new(FakeState::default()));
        let release = release("1.2.3");
        let publisher = make_publisher(
            state.clone(),
            "ghcr.io/owner/charts/project",
            &chart,
            &release.version,
        );

        assert_eq!(publisher.state(&release).unwrap(), PublisherState::Missing);
        assert_eq!(
            publisher.publish(&release).unwrap(),
            [ArtifactStep {
                kind: ArtifactKind::Tag,
                name: "ghcr.io/owner/charts/project:1.2.3".into(),
                step: AssetStep::Uploaded,
            }]
        );
        assert_eq!(
            publisher.state(&release).unwrap(),
            PublisherState::Published
        );

        let writes = state.borrow().writes.clone();
        let manifest = {
            let state = state.borrow();
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
        let config: Value = serde_json::from_slice(&state.borrow().blobs[config_digest]).unwrap();
        assert_eq!(config["name"], "project");
        assert_eq!(config["version"], "1.2.3");
        assert_eq!(config["appVersion"], "1.2.3");

        assert_eq!(
            publisher.publish(&release).unwrap(),
            [ArtifactStep {
                kind: ArtifactKind::Tag,
                name: "ghcr.io/owner/charts/project:1.2.3".into(),
                step: AssetStep::AlreadyUploaded,
            }]
        );
        assert_eq!(state.borrow().writes, writes);
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
        let state = Rc::new(RefCell::new(FakeState::default()));
        let release = release("1.2.3");
        make_publisher(
            state.clone(),
            "ghcr.io/owner/charts/project",
            &first,
            &release.version,
        )
        .publish(&release)
        .unwrap();
        let writes_before = state.borrow().writes.clone();
        let conflicting = make_publisher(
            state.clone(),
            "ghcr.io/owner/charts/project",
            &other,
            &release.version,
        );
        let tag_writes = Rc::new(Cell::new(0));

        let error = publish_ordered(
            &FakeTagPublisher {
                writes: tag_writes.clone(),
            },
            &release,
            &[Box::new(conflicting)],
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(&other.to_string_lossy().to_string()),
            "{error}"
        );
        assert_eq!(tag_writes.get(), 0);
        assert_eq!(state.borrow().writes, writes_before);
    }

    #[test]
    fn an_unknown_remote_state_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let state = Rc::new(RefCell::new(FakeState {
            inspection_fails: true,
            ..FakeState::default()
        }));
        let release = release("1.2.3");
        let publisher = make_publisher(
            state.clone(),
            "ghcr.io/owner/charts/project",
            &chart,
            &release.version,
        );

        let error = publisher.state(&release).unwrap_err().to_string();

        assert!(error.contains("registry unavailable"), "{error}");
        assert!(state.borrow().writes.is_empty());
    }

    #[test]
    fn chart_name_and_version_are_checked_before_publication() {
        let directory = tempfile::tempdir().unwrap();
        let chart = chart(directory.path(), "project", "1.2.3");
        let path = chart.to_string_lossy();
        let wrong_version = checked_chart(
            "ghcr.io/owner/charts/project",
            &path,
            &Version::new(1, 2, 4),
        )
        .unwrap_err()
        .to_string();
        assert!(wrong_version.contains("expected release version 1.2.4"));

        let wrong_name = checked_chart("ghcr.io/owner/charts/other", &path, &Version::new(1, 2, 3))
            .unwrap_err()
            .to_string();
        assert!(wrong_name.contains("must end in chart name \"project\""));
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
