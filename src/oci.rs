//! The OCI publisher: pushes images built with Nix to an OCI registry under the release's tags.
//!
//! Images come as Docker image archives, the output of `dockerTools.buildImage` and
//! `buildLayeredImage` (a tarball, possibly compressed) or `streamLayeredImage` (an executable
//! that writes the archive). Presswerk pushes their layers uncompressed, exactly as the archive holds
//! them, so the manifest and index it pushes depend only on the image content: a retry produces
//! byte-identical manifests with identical digests, without relying on a deterministic
//! compressor.

use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, Publisher,
    PublisherState, Release,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use url::Url;

pub const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
pub const LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar";
/// The index annotation that records the release version, so aliases can be compared.
pub const VERSION_ANNOTATION: &str = "org.opencontainers.image.version";

const VERSION_PLACEHOLDER: &str = "{version}";
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";

/// A registry repository such as `ghcr.io/owner/project`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryReference {
    /// The registry's base URL, such as `https://ghcr.io/`.
    pub registry: Url,
    /// The repository name within the registry, such as `owner/project`.
    pub name: String,
}

impl RepositoryReference {
    /// Parse `host[:port]/path`. The host is required, so an image never lands on a default
    /// registry by accident. Loopback hosts use plain HTTP, like Docker's default insecure
    /// registries; every other registry uses HTTPS.
    pub fn parse(repository: &str) -> Result<Self, String> {
        let invalid = |reason: &str| format!("OCI repository {repository:?} {reason}");
        let (host, name) = repository
            .split_once('/')
            .ok_or_else(|| invalid("must be a registry host followed by a repository path"))?;
        if !(host.contains('.') || host.contains(':') || host == "localhost") {
            return Err(invalid(
                "must start with a registry host, such as ghcr.io/owner/project",
            ));
        }
        // Only a host and port: credentials belong in environment variables, never in the
        // declaration, and a query or fragment has no meaning for a registry.
        if !valid_host(host) {
            return Err(invalid(
                "must start with a plain registry host and optional port, without credentials, query, or fragment",
            ));
        }
        if name.is_empty()
            || !name.split('/').all(|component| {
                !component.is_empty()
                    && component
                        .bytes()
                        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
                    && component.starts_with(|c: char| c.is_ascii_alphanumeric())
                    && component.ends_with(|c: char| c.is_ascii_alphanumeric())
            })
        {
            return Err(invalid(
                "must have a lowercase repository path of letters, digits, and separators",
            ));
        }
        let hostname = host.rsplit_once(':').map_or(host, |(hostname, port)| {
            if port.bytes().all(|byte| byte.is_ascii_digit()) {
                hostname
            } else {
                host
            }
        });
        let scheme = if matches!(hostname, "localhost" | "127.0.0.1" | "[::1]") {
            "http"
        } else {
            "https"
        };
        let (host, name) = if host == "docker.io" {
            let name = if name.contains('/') {
                name.to_owned()
            } else {
                format!("library/{name}")
            };
            ("registry-1.docker.io", name)
        } else {
            (host, name.to_owned())
        };
        let registry = Url::parse(&format!("{scheme}://{host}/"))
            .map_err(|error| invalid(&format!("has an invalid registry host: {error}")))?;
        if registry.path() != "/" || registry.query().is_some() {
            return Err(invalid("has an invalid registry host"));
        }
        Ok(Self { registry, name })
    }
}

/// A registry host: a DNS name, IPv4 address, or bracketed IPv6 address, with an optional port.
fn valid_host(host: &str) -> bool {
    let (name_valid, port) = if let Some(rest) = host.strip_prefix('[') {
        let Some((address, port)) = rest.split_once(']') else {
            return false;
        };
        (
            !address.is_empty()
                && address
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b':'),
            port,
        )
    } else {
        let (name, port) = host
            .find(':')
            .map_or((host, ""), |index| (&host[..index], &host[index..]));
        (
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
            port,
        )
    };
    name_valid
        && (port.is_empty()
            || port.strip_prefix(':').is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            }))
}

/// The platform an image runs on, such as `linux/amd64` or `linux/arm/v7`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl Platform {
    pub fn parse(platform: &str) -> Result<Self, String> {
        let parts: Vec<_> = platform.split('/').collect();
        let valid = |part: &&str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        };
        match parts.as_slice() {
            [os, architecture] | [os, architecture, _]
                if parts.iter().all(valid) && !os.is_empty() && !architecture.is_empty() =>
            {
                Ok(Self {
                    os: (*os).into(),
                    architecture: (*architecture).into(),
                    variant: parts.get(2).map(|variant| (*variant).into()),
                })
            }
            _ => Err(format!(
                "image platform {platform:?} must have the form os/architecture[/variant], such as linux/amd64"
            )),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.os, self.architecture)?;
        if let Some(variant) = &self.variant {
            write!(formatter, "/{variant}")?;
        }
        Ok(())
    }
}

/// One rendered tag of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OciTag {
    /// A tag of this version, such as `1.2.0`; it is never moved once pushed.
    Version(String),
    /// An alias, such as `latest` or `1.2`; it moves to a higher stable version.
    Alias(String),
}

impl OciTag {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Version(tag) | Self::Alias(tag) => tag,
        }
    }
}

fn is_valid_tag(tag: &str) -> bool {
    tag.len() <= 128
        && tag.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// Check tag and alias templates before any version is known.
pub fn validate_tag_templates(tags: &[String], aliases: &[String]) -> Result<(), String> {
    if tags.is_empty() {
        return Err(
            "an OCI publisher needs at least one tag, so every release has a tag of its own".into(),
        );
    }
    if let Some(tag) = tags.iter().find(|tag| !tag.contains(VERSION_PLACEHOLDER)) {
        return Err(format!(
            "OCI tag {tag:?} must contain {{version}}; declare a tag that follows other versions under aliases"
        ));
    }
    if let Some(alias) = aliases
        .iter()
        .find(|alias| alias.contains(VERSION_PLACEHOLDER))
    {
        return Err(format!(
            "OCI alias {alias:?} must not contain {{version}}, which names a single version; declare it under tags"
        ));
    }
    render_tags(tags, aliases, &Version::new(0, 0, 0)).map(|_| ())
}

/// Replace the version placeholders of a template.
fn render_template(template: &str, version: &Version) -> String {
    template
        .replace(VERSION_PLACEHOLDER, &version.to_string())
        .replace("{major}", &version.major.to_string())
        .replace("{minor}", &version.minor.to_string())
        .replace("{patch}", &version.patch.to_string())
}

/// Render the tag and alias templates for a version, tags first.
pub fn render_tags(
    tags: &[String],
    aliases: &[String],
    version: &Version,
) -> Result<Vec<OciTag>, String> {
    if !version.build.is_empty() {
        return Err(format!(
            "OCI tags cannot carry version {version}: OCI tags do not allow `+`, so build metadata cannot be published yet (https://github.com/jkuball/presswerk/issues/89)"
        ));
    }
    let mut seen = HashSet::new();
    let tags = tags.iter().map(|template| {
        (
            template,
            OciTag::Version(render_template(template, version)),
        )
    });
    let aliases = aliases
        .iter()
        .map(|template| (template, OciTag::Alias(render_template(template, version))));
    let mut rendered = Vec::new();
    for (template, tag) in tags.chain(aliases) {
        if !is_valid_tag(tag.name()) {
            return Err(format!(
                "OCI tag {template:?} must render to letters, digits, `_`, `.`, and `-`, not starting with `.` or `-`, and at most 128 characters; the placeholders are {{version}}, {{major}}, {{minor}}, and {{patch}}"
            ));
        }
        if !seen.insert(tag.name().to_owned()) {
            return Err(format!(
                "OCI tag {:?} is declared twice for version {version}",
                tag.name()
            ));
        }
        rendered.push(tag);
    }
    Ok(rendered)
}

fn sha256_digest(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

fn hex_digest(digest: &[u8]) -> String {
    digest
        .iter()
        .fold(String::from("sha256:"), |mut output, byte| {
            use fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
}

/// A blob stored in a file, such as an image layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub path: PathBuf,
    pub digest: String,
    pub size: u64,
}

impl Blob {
    fn from_file(path: PathBuf) -> io::Result<Self> {
        let mut file = File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 16];
        let mut size = 0;
        loop {
            match file.read(&mut buffer)? {
                0 => break,
                read => {
                    hasher.update(&buffer[..read]);
                    size += read as u64;
                }
            }
        }
        Ok(Self {
            path,
            digest: hex_digest(&hasher.finalize()),
            size,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageManifest {
    schema_version: u32,
    media_type: &'static str,
    config: Descriptor,
    layers: Vec<Descriptor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageIndex {
    schema_version: u32,
    media_type: &'static str,
    manifests: Vec<Descriptor>,
    annotations: BTreeMap<String, String>,
}

/// An image read from a Docker image archive and converted to an OCI image manifest.
#[derive(Debug)]
pub struct LocalImage {
    pub platform: Platform,
    pub config: Blob,
    pub layers: Vec<Blob>,
    pub manifest: Vec<u8>,
    pub manifest_digest: String,
    /// Holds the unpacked layers until the image is pushed.
    _directory: tempfile::TempDir,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ArchiveEntry {
    config: String,
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ImageConfig {
    architecture: String,
    os: String,
    #[serde(default)]
    variant: Option<String>,
    rootfs: RootFs,
}

#[derive(Debug, Deserialize)]
struct RootFs {
    diff_ids: Vec<String>,
}

impl LocalImage {
    /// Read a fetched image: an executable that streams a Docker image archive, such as the
    /// output of `dockerTools.streamLayeredImage`, or the archive itself, gzip- or
    /// zstd-compressed or not. The Nix store keeps only the executable bit of a file's mode, so
    /// that bit, not the content, tells the two apart.
    pub fn load(platform: &str, path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let expected = Platform::parse(platform)?;
        let failed = |error: &dyn fmt::Display| {
            format!(
                "could not read image {platform} ({}): {error}",
                path.display()
            )
        };
        let metadata = std::fs::metadata(path).map_err(|error| failed(&error))?;
        if !metadata.is_file() {
            return Err(failed(
                &"not a file; expected an image archive or an executable that streams one",
            ));
        }
        let directory = tempfile::tempdir().map_err(|error| failed(&error))?;

        if metadata.permissions().mode() & 0o111 != 0 {
            let mut child = Command::new(path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|error| failed(&format!("could not run the image executable: {error}")))?;
            let stdout = child.stdout.take().expect("piped stdout");
            let unpacked = unpack(stdout, directory.path());
            let status = child.wait().map_err(|error| failed(&error))?;
            unpacked.map_err(|error| failed(&error))?;
            if !status.success() {
                return Err(failed(&format!(
                    "the image executable exited with {status}"
                )));
            }
            return Self::from_directory(&expected, directory).map_err(|error| failed(&error));
        }

        let mut file = File::open(path).map_err(|error| failed(&error))?;
        let mut magic = [0; 4];
        let read = read_prefix(&mut file, &mut magic).map_err(|error| failed(&error))?;
        let magic = &magic[..read];
        file.rewind().map_err(|error| failed(&error))?;
        if magic.starts_with(&[0x1f, 0x8b]) {
            unpack(
                flate2::read::GzDecoder::new(BufReader::new(file)),
                directory.path(),
            )
            .map_err(|error| failed(&error))?;
        } else if magic == [0x28, 0xb5, 0x2f, 0xfd] {
            let decoder = ruzstd::decoding::StreamingDecoder::new(BufReader::new(file))
                .map_err(|error| failed(&error))?;
            unpack(decoder, directory.path()).map_err(|error| failed(&error))?;
        } else {
            unpack(BufReader::new(file), directory.path()).map_err(|error| failed(&error))?;
        }

        Self::from_directory(&expected, directory).map_err(|error| failed(&error))
    }

    fn from_directory(expected: &Platform, directory: tempfile::TempDir) -> Result<Self, String> {
        let root = directory.path();
        let entries: Vec<ArchiveEntry> = serde_json::from_slice(
            &std::fs::read(root.join("manifest.json"))
                .map_err(|error| format!("the archive has no readable manifest.json: {error}"))?,
        )
        .map_err(|error| format!("invalid manifest.json: {error}"))?;
        let [entry] = entries.as_slice() else {
            return Err(format!(
                "the archive must hold exactly one image, found {}",
                entries.len()
            ));
        };
        let member = |name: &str| -> Result<PathBuf, String> {
            let relative = Path::new(name);
            if relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            {
                Ok(root.join(relative))
            } else {
                Err(format!("the archive refers to {name:?} outside itself"))
            }
        };
        let config_path = member(&entry.config)?;
        let config_bytes = std::fs::read(&config_path).map_err(|error| {
            format!("could not read the image config {}: {error}", entry.config)
        })?;
        let config: ImageConfig = serde_json::from_slice(&config_bytes)
            .map_err(|error| format!("invalid image config {}: {error}", entry.config))?;
        let diff_ids = config.rootfs.diff_ids;
        let platform = Platform {
            architecture: config.architecture,
            os: config.os,
            variant: config.variant.filter(|variant| !variant.is_empty()),
        };
        if platform != *expected {
            return Err(format!(
                "the image is built for {platform}, but is declared as {expected}"
            ));
        }
        let config = Blob {
            path: config_path,
            digest: sha256_digest(&config_bytes),
            size: config_bytes.len() as u64,
        };
        let layers = entry
            .layers
            .iter()
            .map(|layer| {
                Blob::from_file(member(layer)?)
                    .map_err(|error| format!("could not read layer {layer}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let digests: Vec<_> = layers.iter().map(|layer| &layer.digest).collect();
        if digests.iter().copied().ne(diff_ids.iter()) {
            return Err(
                "the layers do not match the image config's diff_ids; Presswerk pushes layers uncompressed and needs an archive with uncompressed layers"
                    .into(),
            );
        }

        let descriptor = |media_type: &str, blob: &Blob| Descriptor {
            media_type: media_type.into(),
            digest: blob.digest.clone(),
            size: blob.size,
            platform: None,
        };
        let manifest = serde_json::to_vec(&ImageManifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE,
            config: descriptor(CONFIG_MEDIA_TYPE, &config),
            layers: layers
                .iter()
                .map(|layer| descriptor(LAYER_MEDIA_TYPE, layer))
                .collect(),
        })
        .expect("serialize an image manifest");
        let manifest_digest = sha256_digest(&manifest);
        Ok(Self {
            platform,
            config,
            layers,
            manifest,
            manifest_digest,
            _directory: directory,
        })
    }

    /// The image's descriptor in an index.
    #[must_use]
    pub fn descriptor(&self) -> Descriptor {
        Descriptor {
            media_type: MANIFEST_MEDIA_TYPE.into(),
            digest: self.manifest_digest.clone(),
            size: self.manifest.len() as u64,
            platform: Some(self.platform.clone()),
        }
    }

    /// The config and every distinct layer.
    fn blobs(&self) -> Vec<&Blob> {
        let mut seen = HashSet::new();
        std::iter::once(&self.config)
            .chain(&self.layers)
            .filter(|blob| seen.insert(&blob.digest))
            .collect()
    }
}

fn read_prefix(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut read = 0;
    while read < buffer.len() {
        match file.read(&mut buffer[read..])? {
            0 => break,
            count => read += count,
        }
    }
    Ok(read)
}

fn unpack(reader: impl Read, directory: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    archive.set_unpack_xattrs(false);
    archive.unpack(directory)?;
    // Read to the end, so a streaming executable is not cut off after the end-of-archive marker.
    io::copy(&mut archive.into_inner(), &mut io::sink())?;
    Ok(())
}

/// The image index for a release: one manifest per platform, annotated with the version.
#[must_use]
pub fn image_index(images: &[LocalImage], version: &Version) -> (Vec<u8>, String) {
    let mut manifests: Vec<_> = images.iter().map(LocalImage::descriptor).collect();
    manifests.sort_by(|left, right| left.platform.cmp(&right.platform));
    let index = serde_json::to_vec(&ImageIndex {
        schema_version: 2,
        media_type: INDEX_MEDIA_TYPE,
        manifests,
        annotations: BTreeMap::from([(VERSION_ANNOTATION.into(), version.to_string())]),
    })
    .expect("serialize an image index");
    let digest = sha256_digest(&index);
    (index, digest)
}

/// Registry credentials, read from the environment at run time.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credentials([redacted])")
    }
}

impl Credentials {
    pub fn from_environment(username: &str, password: &str) -> Result<Self, PublicationError> {
        let read = |name: &str| {
            let value = env::var(name).map_err(|_| {
                PublicationError::new(format!(
                    "OCI credential environment variable {name:?} is not set"
                ))
            })?;
            if value.is_empty() {
                return Err(PublicationError::new(format!(
                    "OCI credential environment variable {name:?} is empty"
                )));
            }
            Ok(value)
        };
        Ok(Self {
            username: read(username)?,
            password: read(password)?,
        })
    }
}

enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    File(&'a Blob),
}

enum CallError {
    Status(u16),
    Transport(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(formatter, "HTTP {status}"),
            Self::Transport(error) => formatter.write_str(error),
        }
    }
}

/// A client for one repository of an OCI distribution registry.
pub struct RegistryClient {
    agent: ureq::Agent,
    repository: RepositoryReference,
    display: String,
    /// `None` reads anonymously, for repositories anyone may pull from.
    credentials: Option<Credentials>,
    authorization: Mutex<Option<String>>,
}

impl fmt::Debug for RegistryClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryClient")
            .field("repository", &self.display)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

impl RegistryClient {
    /// A client for `repository`; without credentials it only pulls, anonymously.
    pub fn new(
        repository: &str,
        credentials: Option<Credentials>,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            repository: RepositoryReference::parse(repository).map_err(PublicationError::new)?,
            display: repository.into(),
            credentials,
            authorization: Mutex::new(None),
        })
    }

    fn url(&self, kind: &str, reference: &str) -> Url {
        self.repository
            .registry
            .join(&format!("v2/{}/{kind}/{reference}", self.repository.name))
            .expect("registry URLs are valid")
    }

    fn call(
        &self,
        method: &str,
        url: &Url,
        headers: &[(&str, &str)],
        body: &Body<'_>,
    ) -> Result<ureq::Response, CallError> {
        let mut authenticated = false;
        loop {
            let mut request = self
                .agent
                .request(method, url.as_str())
                .set("User-Agent", "presswerk");
            for (name, value) in headers {
                request = request.set(name, value);
            }
            if let Some(authorization) = self.authorization.lock().expect("lock").as_deref() {
                request = request.set("Authorization", authorization);
            }
            let result = match body {
                Body::Empty => request.call(),
                Body::Bytes(bytes) => request.send_bytes(bytes),
                Body::File(blob) => {
                    let file = File::open(&blob.path).map_err(|error| {
                        CallError::Transport(format!(
                            "could not read {}: {error}",
                            blob.path.display()
                        ))
                    })?;
                    request
                        .set("Content-Length", &blob.size.to_string())
                        .send(BufReader::new(file))
                }
            };
            match result {
                Ok(response) => return Ok(response),
                Err(ureq::Error::Status(401, response)) if !authenticated => {
                    authenticated = true;
                    let challenge = response.header("WWW-Authenticate").unwrap_or_default();
                    let authorization = self.authenticate(challenge)?;
                    *self.authorization.lock().expect("lock") = Some(authorization);
                }
                Err(ureq::Error::Status(status @ (401 | 403), _)) if self.credentials.is_none() => {
                    return Err(self.not_public(&format!("the registry answered HTTP {status}")));
                }
                Err(ureq::Error::Status(status, _)) => return Err(CallError::Status(status)),
                Err(ureq::Error::Transport(error)) => {
                    return Err(CallError::Transport(error.to_string()));
                }
            }
        }
    }

    fn not_public(&self, reason: &str) -> CallError {
        CallError::Transport(format!(
            "{reason} to an anonymous read, so {} does not seem to be publicly readable; is `public = true` correct?",
            self.display
        ))
    }

    /// Answer a registry's authentication challenge with the configured credentials, or
    /// anonymously with pull access only.
    fn authenticate(&self, challenge: &str) -> Result<String, CallError> {
        let basic = self.credentials.as_ref().map(|credentials| {
            format!(
                "Basic {}",
                base64(format!("{}:{}", credentials.username, credentials.password))
            )
        });
        let (scheme, parameters) = challenge.split_once(' ').unwrap_or((challenge, ""));
        if scheme.eq_ignore_ascii_case("basic") {
            return basic.ok_or_else(|| self.not_public("the registry asked for a password"));
        }
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(CallError::Transport(format!(
                "registry asked for unsupported authentication {scheme:?}"
            )));
        }
        let parameters = challenge_parameters(parameters);
        let realm = parameters.get("realm").ok_or_else(|| {
            CallError::Transport("registry sent a bearer challenge without a realm".into())
        })?;
        let mut url = Url::parse(realm).map_err(|error| {
            CallError::Transport(format!("registry sent an invalid token realm: {error}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = parameters.get("service") {
                query.append_pair("service", service);
            }
            let actions = if basic.is_some() { "pull,push" } else { "pull" };
            query.append_pair(
                "scope",
                &format!("repository:{}:{actions}", self.repository.name),
            );
        }
        let mut request = self.agent.get(url.as_str()).set("User-Agent", "presswerk");
        if let Some(basic) = &basic {
            request = request.set("Authorization", basic);
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) if basic.is_none() => {
                return Err(self.not_public(&format!(
                    "the registry token service answered HTTP {status}"
                )));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(CallError::Transport(format!(
                    "registry token service rejected the credentials (HTTP {status})"
                )));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(CallError::Transport(format!(
                    "could not reach the registry token service: {error}"
                )));
            }
        };
        #[derive(Deserialize)]
        struct TokenResponse {
            token: Option<String>,
            access_token: Option<String>,
        }
        let token: TokenResponse = response.into_json().map_err(|error| {
            CallError::Transport(format!("invalid registry token response: {error}"))
        })?;
        token
            .token
            .or(token.access_token)
            .filter(|token| !token.is_empty())
            .map(|token| format!("Bearer {token}"))
            .ok_or_else(|| CallError::Transport("registry token response has no token".into()))
    }

    fn unknown(&self, what: &str, error: &CallError) -> PublicationError {
        PublicationError::new(format!(
            "could not inspect {what} in {}: {error}; Presswerk cannot tell whether it exists",
            self.display
        ))
    }

    /// The digest a tag or digest reference resolves to, or `None` if the registry has none.
    pub fn manifest_digest(&self, reference: &str) -> Result<Option<String>, PublicationError> {
        let url = self.url("manifests", reference);
        match self.call("HEAD", &url, &[("Accept", MANIFEST_ACCEPT)], &Body::Empty) {
            Ok(response) => match response.header("Docker-Content-Digest") {
                Some(digest) if digest.starts_with("sha256:") => Ok(Some(digest.to_owned())),
                _ => Ok(self.manifest(reference)?.map(|(digest, _)| digest)),
            },
            Err(CallError::Status(404)) => Ok(None),
            Err(error) => Err(self.unknown(&format!("manifest {reference}"), &error)),
        }
    }

    /// The manifest a reference resolves to, with the digest of its bytes.
    pub fn manifest(&self, reference: &str) -> Result<Option<(String, Vec<u8>)>, PublicationError> {
        let url = self.url("manifests", reference);
        match self.call("GET", &url, &[("Accept", MANIFEST_ACCEPT)], &Body::Empty) {
            Ok(response) => {
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .take(16 * 1024 * 1024)
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        self.unknown(
                            &format!("manifest {reference}"),
                            &CallError::Transport(error.to_string()),
                        )
                    })?;
                Ok(Some((sha256_digest(&bytes), bytes)))
            }
            Err(CallError::Status(404)) => Ok(None),
            Err(error) => Err(self.unknown(&format!("manifest {reference}"), &error)),
        }
    }

    pub fn blob_exists(&self, digest: &str) -> Result<bool, PublicationError> {
        match self.call("HEAD", &self.url("blobs", digest), &[], &Body::Empty) {
            Ok(_) => Ok(true),
            Err(CallError::Status(404)) => Ok(false),
            Err(error) => Err(self.unknown(&format!("blob {digest}"), &error)),
        }
    }

    /// Upload a blob in one request; a blob that appears meanwhile counts as uploaded.
    pub fn upload_blob(&self, blob: &Blob) -> Result<(), PublicationError> {
        let failure = match self.start_upload() {
            Ok(mut location) => {
                location
                    .query_pairs_mut()
                    .append_pair("digest", &blob.digest);
                match self.call(
                    "PUT",
                    &location,
                    &[("Content-Type", "application/octet-stream")],
                    &Body::File(blob),
                ) {
                    Ok(_) => return Ok(()),
                    Err(error) => error.to_string(),
                }
            }
            Err(error) => error,
        };
        if self.blob_exists(&blob.digest)? {
            return Ok(());
        }
        Err(PublicationError::new(format!(
            "could not upload blob {} to {}: {failure}",
            blob.digest, self.display
        )))
    }

    fn start_upload(&self) -> Result<Url, String> {
        let url = self
            .repository
            .registry
            .join(&format!("v2/{}/blobs/uploads/", self.repository.name))
            .expect("registry URLs are valid");
        let response = self
            .call("POST", &url, &[], &Body::Bytes(&[]))
            .map_err(|error| error.to_string())?;
        let location = response
            .header("Location")
            .ok_or("the registry started an upload without a Location")?;
        url.join(location)
            .map_err(|error| format!("the registry sent an invalid upload location: {error}"))
    }

    /// Put a manifest under a tag or digest; a failed write is inspected again, and succeeds
    /// only if the reference now holds exactly this manifest.
    pub fn put_manifest(
        &self,
        reference: &str,
        media_type: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<(), PublicationError> {
        let url = self.url("manifests", reference);
        let failure = match self.call(
            "PUT",
            &url,
            &[("Content-Type", media_type)],
            &Body::Bytes(bytes),
        ) {
            Ok(response) => match response.header("Docker-Content-Digest") {
                Some(actual) if actual != digest => {
                    return Err(PublicationError::new(format!(
                        "{} stored {reference} with digest {actual}, expected {digest}",
                        self.display
                    )));
                }
                _ => return Ok(()),
            },
            Err(error) => error.to_string(),
        };
        if self.manifest_digest(reference)?.as_deref() == Some(digest) {
            return Ok(());
        }
        Err(PublicationError::new(format!(
            "could not push {}:{reference}: {failure}",
            self.display
        )))
    }
}

fn challenge_parameters(parameters: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut rest = parameters.trim();
    while let Some((key, value)) = rest.split_once('=') {
        let key = key
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let value = value.trim_start();
        let (value, remainder) = if let Some(quoted) = value.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            (&quoted[..end], quoted.get(end + 1..).unwrap_or(""))
        } else {
            let end = value.find(',').unwrap_or(value.len());
            (&value[..end], &value[end..])
        };
        result.insert(key, value.to_owned());
        rest = remainder.trim_start_matches([',', ' ']);
    }
    result
}

fn base64(input: impl AsRef<[u8]>) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.as_ref();
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                output.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                output.push('=');
            }
        }
    }
    output
}

/// What a registry holds under one tag, compared with what this release pushes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagState {
    Missing,
    Matching,
    /// An alias that points to a lower version and moves to this one.
    Behind,
    /// An alias that is left alone, with the reason.
    Kept(String),
    /// An alias whose state could not be read; it is left alone and repaired by a later run.
    Unknown(String),
    Conflicting(String),
}

/// Pushes one image index per release and points every tag at it.
#[derive(Debug)]
pub struct OciPublisher {
    registry: RegistryClient,
    repository: String,
    tags: Vec<String>,
    aliases: Vec<String>,
    images: Vec<LocalImage>,
}

impl OciPublisher {
    /// Build the publisher from its declaration; its images must already be fetched.
    pub fn from_declaration(
        declaration: &crate::declaration::OciPublisher,
    ) -> Result<Self, PublicationError> {
        let credentials = Credentials::from_environment(
            &declaration.username.secret,
            &declaration.password.secret,
        )?;
        let images = declaration
            .platforms
            .iter()
            .map(|(platform, path)| LocalImage::load(platform, path))
            .collect::<Result<Vec<_>, _>>()
            .map_err(PublicationError::new)?;
        Ok(Self::new(
            RegistryClient::new(&declaration.repository, Some(credentials))?,
            &declaration.repository,
            declaration.tags.clone(),
            declaration.aliases.clone(),
            images,
        ))
    }

    pub fn new(
        registry: RegistryClient,
        repository: &str,
        tags: Vec<String>,
        aliases: Vec<String>,
        images: Vec<LocalImage>,
    ) -> Self {
        Self {
            registry,
            repository: repository.into(),
            tags,
            aliases,
            images,
        }
    }

    fn reference(&self, tag: &str) -> String {
        format!("{}:{tag}", self.repository)
    }

    fn inspect(&self, release: &Release) -> Result<Inspection, PublicationError> {
        let tags = render_tags(&self.tags, &self.aliases, &release.version)
            .map_err(PublicationError::new)?;
        let (index, digest) = image_index(&self.images, &release.version);
        let states = tags
            .into_iter()
            .map(|tag| {
                let state = self.tag_state(&tag, release, &digest)?;
                Ok((tag, state))
            })
            .collect::<Result<_, PublicationError>>()?;
        Ok(Inspection {
            index,
            digest,
            tags: states,
        })
    }

    fn tag_state(
        &self,
        tag: &OciTag,
        release: &Release,
        digest: &str,
    ) -> Result<TagState, PublicationError> {
        match tag {
            OciTag::Version(name) => Ok(match self.registry.manifest_digest(name)? {
                None => TagState::Missing,
                Some(actual) if actual == digest => TagState::Matching,
                Some(actual) => TagState::Conflicting(format!(
                    "{} points to {actual}, expected {digest}",
                    self.reference(name)
                )),
            }),
            OciTag::Alias(name) => {
                let prerelease = !release.version.pre.is_empty();
                // Aliases are best effort: an alias that cannot be read is reported, not fatal.
                let current = match self.registry.manifest(name) {
                    Ok(current) => current,
                    Err(error) => return Ok(TagState::Unknown(error.to_string())),
                };
                let Some((actual, manifest)) = current else {
                    return Ok(if prerelease {
                        TagState::Kept("pre-release".into())
                    } else {
                        TagState::Missing
                    });
                };
                if actual == digest {
                    return Ok(TagState::Matching);
                }
                if prerelease {
                    return Ok(TagState::Kept("pre-release".into()));
                }
                Ok(match annotated_version(&manifest) {
                    None => TagState::Kept(format!("points to {actual} without a version")),
                    Some(current) if current > release.version => {
                        TagState::Kept(format!("{current} is higher"))
                    }
                    Some(current) if current == release.version => TagState::Conflicting(format!(
                        "{} points to {actual} for version {current}, expected {digest}",
                        self.reference(name)
                    )),
                    Some(_) => TagState::Behind,
                })
            }
        }
    }

    /// Push every blob and manifest of an image that the registry does not have yet.
    fn push_image(&self, image: &LocalImage) -> Result<AssetStep, PublicationError> {
        let mut pushed = false;
        for blob in image.blobs() {
            if !self.registry.blob_exists(&blob.digest)? {
                self.registry.upload_blob(blob)?;
                pushed = true;
            }
        }
        if self
            .registry
            .manifest_digest(&image.manifest_digest)?
            .is_none()
        {
            self.registry.put_manifest(
                &image.manifest_digest,
                MANIFEST_MEDIA_TYPE,
                &image.manifest,
                &image.manifest_digest,
            )?;
            pushed = true;
        }
        Ok(if pushed {
            AssetStep::Uploaded
        } else {
            AssetStep::AlreadyUploaded
        })
    }

    /// Point an alias at the index if it still needs to move; the error is the warning text.
    fn move_alias(
        &self,
        tag: &OciTag,
        release: &Release,
        inspection: &Inspection,
    ) -> Result<(), String> {
        match self
            .tag_state(tag, release, &inspection.digest)
            .map_err(|error| error.to_string())?
        {
            TagState::Missing | TagState::Behind => self
                .registry
                .put_manifest(
                    tag.name(),
                    INDEX_MEDIA_TYPE,
                    &inspection.index,
                    &inspection.digest,
                )
                .map_err(|error| error.to_string()),
            TagState::Unknown(reason) | TagState::Conflicting(reason) => Err(reason),
            TagState::Matching | TagState::Kept(_) => Ok(()),
        }
    }

    fn steps(&self, inspection: &Inspection, images: Vec<AssetStep>) -> ArtifactSteps {
        let mut steps: ArtifactSteps = self
            .images
            .iter()
            .zip(images)
            .map(|(image, step)| ArtifactStep {
                kind: ArtifactKind::Image,
                name: image.platform.to_string(),
                step,
            })
            .collect();
        steps.extend(inspection.tags.iter().map(|(tag, state)| ArtifactStep {
            kind: match tag {
                OciTag::Version(_) => ArtifactKind::Tag,
                OciTag::Alias(_) => ArtifactKind::Alias,
            },
            name: self.reference(tag.name()),
            step: match state {
                TagState::Kept(reason) => AssetStep::Kept(reason.clone()),
                TagState::Unknown(reason) => AssetStep::Warning(reason.clone()),
                _ => AssetStep::AlreadyUploaded,
            },
        }));
        steps
    }
}

#[derive(Debug)]
struct Inspection {
    index: Vec<u8>,
    digest: String,
    tags: Vec<(OciTag, TagState)>,
}

impl Inspection {
    fn conflict(&self) -> Option<&str> {
        self.tags.iter().find_map(|(_, state)| match state {
            TagState::Conflicting(message) => Some(message.as_str()),
            _ => None,
        })
    }
}

/// The version annotated on an OCI image index, as Presswerk pushes it; `None` for any other
/// manifest, so only an index can complete a release or hold an alias back.
pub(crate) fn annotated_version(manifest: &[u8]) -> Option<Version> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Annotated {
        schema_version: u32,
        media_type: String,
        #[allow(dead_code)]
        manifests: Vec<serde_json::Value>,
        #[serde(default)]
        annotations: BTreeMap<String, String>,
    }
    let annotated: Annotated = serde_json::from_slice(manifest).ok()?;
    if annotated.schema_version != 2 || annotated.media_type != INDEX_MEDIA_TYPE {
        return None;
    }
    Version::parse(annotated.annotations.get(VERSION_ANNOTATION)?).ok()
}

impl Publisher for OciPublisher {
    fn state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        let inspection = self.inspect(release)?;
        if let Some(conflict) = inspection.conflict() {
            return Ok(PublisherState::Conflicting(conflict.into()));
        }
        let version_tags: Vec<_> = inspection
            .tags
            .iter()
            .filter(|(tag, _)| matches!(tag, OciTag::Version(_)))
            .collect();
        Ok(
            if version_tags
                .iter()
                .all(|(_, state)| *state == TagState::Missing)
            {
                PublisherState::Missing
            } else if !version_tags
                .iter()
                .all(|(_, state)| *state == TagState::Matching)
            {
                PublisherState::Incomplete
            } else if inspection
                .tags
                .iter()
                .all(|(_, state)| matches!(state, TagState::Matching | TagState::Kept(_)))
            {
                PublisherState::Published
            } else {
                // The release is complete; only best-effort aliases still need to move.
                PublisherState::AliasesPending
            },
        )
    }

    fn publish(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        let inspection = self.inspect(release)?;
        if let Some(conflict) = inspection.conflict() {
            return Err(PublicationError::new(conflict));
        }
        let images = self
            .images
            .iter()
            .map(|image| self.push_image(image))
            .collect::<Result<Vec<_>, _>>()?;
        if self.registry.manifest_digest(&inspection.digest)?.is_none() {
            self.registry.put_manifest(
                &inspection.digest,
                INDEX_MEDIA_TYPE,
                &inspection.index,
                &inspection.digest,
            )?;
        }

        let mut steps = self.steps(&inspection, images);
        let tag_steps = steps.len() - inspection.tags.len();
        for ((tag, state), step) in inspection.tags.iter().zip(&mut steps[tag_steps..]) {
            if !matches!(
                state,
                TagState::Missing | TagState::Behind | TagState::Unknown(_)
            ) {
                continue;
            }
            if let OciTag::Alias(_) = tag {
                // Aliases come last and are best effort: a failure is a warning that a rerun
                // repairs, and never stops the release.
                if let Err(reason) = self.move_alias(tag, release, &inspection) {
                    step.step = AssetStep::Warning(reason);
                } else if let Ok(state) = self.tag_state(tag, release, &inspection.digest) {
                    step.step = match state {
                        TagState::Matching => AssetStep::Uploaded,
                        TagState::Kept(reason) => AssetStep::Kept(reason),
                        TagState::Unknown(reason) | TagState::Conflicting(reason) => {
                            AssetStep::Warning(reason)
                        }
                        TagState::Missing | TagState::Behind => {
                            AssetStep::Warning("the registry did not keep the alias".into())
                        }
                    };
                }
                continue;
            }
            // Look again right before writing: a concurrent run may have pushed the tag, and a
            // tag that now points elsewhere must not be overwritten. A tag that was missing is
            // reported as pushed even if it reappeared meanwhile, such as when a registry restores
            // a deleted version's tags once its index is pushed again.
            match self.tag_state(tag, release, &inspection.digest)? {
                TagState::Missing | TagState::Behind => {
                    self.registry.put_manifest(
                        tag.name(),
                        INDEX_MEDIA_TYPE,
                        &inspection.index,
                        &inspection.digest,
                    )?;
                    step.step = AssetStep::Uploaded;
                }
                TagState::Matching => step.step = AssetStep::Uploaded,
                TagState::Kept(reason) => step.step = AssetStep::Kept(reason),
                TagState::Conflicting(message) | TagState::Unknown(message) => {
                    return Err(PublicationError::new(message));
                }
            }
        }

        match self.state(release)? {
            PublisherState::Published | PublisherState::AliasesPending => Ok(steps),
            state => Err(PublicationError::new(format!(
                "{} was not published: {state:?}",
                self.repository
            ))),
        }
    }

    fn complete_steps(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        let inspection = self.inspect(release)?;
        let images = vec![AssetStep::AlreadyUploaded; self.images.len()];
        Ok(self.steps(&inspection, images))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_repositories() {
        let ghcr = RepositoryReference::parse("ghcr.io/owner/project").unwrap();
        assert_eq!(ghcr.registry.as_str(), "https://ghcr.io/");
        assert_eq!(ghcr.name, "owner/project");

        let local = RepositoryReference::parse("localhost:5000/team/image").unwrap();
        assert_eq!(local.registry.as_str(), "http://localhost:5000/");
        let loopback = RepositoryReference::parse("127.0.0.1:5000/image").unwrap();
        assert_eq!(loopback.registry.as_str(), "http://127.0.0.1:5000/");

        let hub = RepositoryReference::parse("docker.io/nginx").unwrap();
        assert_eq!(hub.registry.as_str(), "https://registry-1.docker.io/");
        assert_eq!(hub.name, "library/nginx");

        let ipv6 = RepositoryReference::parse("[::1]:5000/image").unwrap();
        assert_eq!(ipv6.registry.as_str(), "http://[::1]:5000/");

        let gar = RepositoryReference::parse("europe-docker.pkg.dev/project/repo/image").unwrap();
        assert_eq!(gar.name, "project/repo/image");

        for invalid in [
            "owner/project",
            "ghcr.io",
            "ghcr.io/",
            "ghcr.io/Owner/project",
            "ghcr.io/owner//project",
            "ghcr.io/owner/project:tag",
            "ghcr.io/-owner/project",
            "user:secret@ghcr.io/owner/project",
            "token@ghcr.io/owner/project",
            "ghcr.io?x=1/owner/project",
            "ghcr.io#fragment/owner/project",
            "ghcr.io:port/owner/project",
            "ghcr.io/owner/project?x=1",
            "ghcr.io/owner/project#fragment",
        ] {
            assert!(RepositoryReference::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_platforms() {
        assert_eq!(
            Platform::parse("linux/arm/v7").unwrap().to_string(),
            "linux/arm/v7"
        );
        assert_eq!(Platform::parse("linux/amd64").unwrap().variant, None);
        for invalid in ["linux", "linux/", "Linux/amd64", "linux/amd64/v8/x"] {
            assert!(Platform::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn renders_tags_before_aliases_with_every_placeholder() {
        let tags = render_tags(
            &["{version}".into(), "v{version}-alpine".into()],
            &["latest".into(), "{major}".into(), "{major}.{minor}".into()],
            &Version::parse("1.2.3-rc.1").unwrap(),
        )
        .unwrap();
        assert_eq!(
            tags,
            [
                OciTag::Version("1.2.3-rc.1".into()),
                OciTag::Version("v1.2.3-rc.1-alpine".into()),
                OciTag::Alias("latest".into()),
                OciTag::Alias("1".into()),
                OciTag::Alias("1.2".into()),
            ]
        );
    }

    #[test]
    fn rejects_invalid_tags_and_build_metadata() {
        let strings =
            |templates: &[&str]| templates.iter().map(|&t| t.to_owned()).collect::<Vec<_>>();
        let validate = |tags: &[&str], aliases: &[&str]| {
            validate_tag_templates(&strings(tags), &strings(aliases))
        };
        assert!(validate(&[], &["latest"]).is_err());
        assert!(validate(&["latest"], &[]).is_err());
        assert!(validate(&["{version}"], &["{version}-slim"]).is_err());
        assert!(validate(&["{version}"], &["{unknown}"]).is_err());
        assert!(validate(&["{version}", "{version}"], &[]).is_err());
        // Renders to the version tag of every stable release.
        assert!(validate(&["{version}"], &["{major}.{minor}.{patch}"]).is_err());
        assert!(validate(&["-{version}"], &[]).is_err());
        validate(&["{version}"], &["latest", "{major}", "{major}.{minor}"]).unwrap();

        let error = render_tags(
            &["{version}".into()],
            &[],
            &Version::parse("1.2.3+build.5").unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("issues/89"), "{error}");
    }

    #[test]
    fn parses_authentication_challenges() {
        let parameters = challenge_parameters(
            r#"realm="https://ghcr.io/token",service="ghcr.io",scope="repository:a/b:pull,push""#,
        );
        assert_eq!(parameters["realm"], "https://ghcr.io/token");
        assert_eq!(parameters["service"], "ghcr.io");
        assert_eq!(parameters["scope"], "repository:a/b:pull,push");
    }

    #[test]
    fn encodes_basic_credentials() {
        assert_eq!(base64("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64("a"), "YQ==");
        assert_eq!(base64("ab"), "YWI=");
        assert_eq!(base64(""), "");
    }

    #[test]
    fn credentials_never_appear_in_debug_output() {
        let credentials = Credentials {
            username: "user".into(),
            password: "secret-token".into(),
        };
        assert!(!format!("{credentials:?}").contains("secret-token"));
    }
}
