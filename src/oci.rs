//! The OCI publisher: pushes images built with Nix to an OCI registry under the release's tags.
//!
//! Images come as Docker image archives, the output of `dockerTools.buildImage` and
//! `buildLayeredImage` (a tarball, possibly compressed) or `streamLayeredImage` (an executable
//! that writes the archive). Presswerk compresses their layers with gzip or zstd, as configured,
//! and writes the OCI manifests itself. Each manifest in the index records the Nix store path of
//! its image, so a retry recognizes the release's index by its store paths, whatever bytes the
//! compressor produced.

use crate::declaration::Compression;
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
use std::sync::{Arc, Mutex};
use tempfile::SpooledData;
use url::Url;

pub const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
pub const GZIP_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub const ZSTD_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+zstd";
/// The index annotation that records the release version, so aliases can be compared.
pub const VERSION_ANNOTATION: &str = "org.opencontainers.image.version";
/// The annotation on each manifest in an index that records the Nix store path of its image, so a
/// later run recognizes the release's index without comparing compressed bytes.
pub const STORE_PATH_ANNOTATION: &str = "dev.presswerk.store-path";

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

    /// The registry and name that identify the repository, however it was spelled: hosts are
    /// lowercase without a default port, and Docker Hub's hosts and its `library/` shorthand are
    /// one.
    #[must_use]
    pub fn identity(&self) -> String {
        let host = self.registry.host_str().unwrap_or_default();
        let port = self
            .registry
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        if port.is_empty()
            && matches!(
                host,
                "docker.io" | "index.docker.io" | "registry-1.docker.io"
            )
        {
            if self.name.contains('/') {
                format!("docker.io/{}", self.name)
            } else {
                format!("docker.io/library/{}", self.name)
            }
        } else {
            format!("{host}{port}/{}", self.name)
        }
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
    // Distinct parts, so `{major}` and `{minor}` differ while `{major}.{minor}.{patch}` still
    // collides with `{version}`.
    render_tags(tags, aliases, &Version::new(1, 2, 3)).map(|_| ())
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

/// A blob of an image, as it is uploaded: a compressed layer or the config.
#[derive(Debug)]
pub struct Blob<'a> {
    pub digest: String,
    pub size: u64,
    pub data: BlobData<'a>,
}

#[derive(Debug)]
pub enum BlobData<'a> {
    /// A compressed layer too large to keep in memory, in an unnamed temporary file that is gone
    /// once the blob is handled.
    File(&'a File),
    Bytes(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
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

/// An image read from a Docker image archive and converted to an OCI image manifest with
/// compressed layers, whose blobs were handed on while the archive was read.
#[derive(Debug)]
pub struct Image {
    pub platform: Platform,
    /// Where the image was read from, recorded in the index.
    pub store_path: String,
    pub manifest: Vec<u8>,
    pub manifest_digest: String,
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
}

/// The largest compressed layer kept in memory; a larger one goes to a temporary file.
const SPOOL_LIMIT: usize = 64 << 20;

/// How much of an archive's JSON files, such as `manifest.json` and the image config, is kept in
/// memory, in all.
const METADATA_LIMIT: u64 = 16 << 20;

/// A file of an image archive: a layer, handed on compressed, or a JSON file, kept.
#[derive(Debug, Clone)]
enum Member {
    Layer { digest: String, size: u64 },
    Metadata(Arc<[u8]>),
}

/// The files of an image archive, by their normalized path in it.
#[derive(Debug, Default)]
struct Members {
    files: BTreeMap<String, Member>,
    /// Symbolic links, to the normalized path they name.
    links: BTreeMap<String, String>,
}

enum ReadError {
    Archive(io::Error),
    /// A blob could not be handed on.
    Blob(String),
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Archive(error)
    }
}

impl Members {
    /// Read an archive, handing every layer on as soon as it is compressed, so at most one
    /// compressed layer is on disk at a time. Which files are layers is known only from
    /// `manifest.json`, which may come last, so JSON files are kept and every other file is
    /// treated as a layer; a Docker image archive holds nothing else.
    fn read(
        reader: impl Read,
        compression: Compression,
        put: &mut dyn FnMut(&Blob<'_>) -> Result<(), String>,
    ) -> Result<Self, ReadError> {
        let mut members = Self::default();
        let mut metadata_left = METADATA_LIMIT;
        let mut archive = tar::Archive::new(reader);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let Some(name) = normalize(&entry.path()?) else {
                continue;
            };
            // A later entry replaces an earlier one of the same name, whatever either is.
            members.files.remove(&name);
            members.links.remove(&name);
            let kind = entry.header().entry_type();
            if kind.is_symlink() || kind.is_hard_link() {
                let target = entry.link_name()?.and_then(|target| {
                    if kind.is_symlink() {
                        normalize(
                            &Path::new(&name)
                                .parent()
                                .unwrap_or(Path::new(""))
                                .join(target),
                        )
                    } else {
                        normalize(&target)
                    }
                });
                match target {
                    // A hard link is the file its target is now, not what that name holds later.
                    // A hard link to a symbolic link stays unresolved: its target would be read
                    // relative to the hard link, which no image needs.
                    Some(target) if kind.is_hard_link() => {
                        if let Some(member) = members.files.get(&target) {
                            let member = member.clone();
                            members.files.insert(name, member);
                        }
                    }
                    Some(target) => {
                        members.links.insert(name, target);
                    }
                    None => {}
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let member = if name.ends_with(".json") || name == "repositories" {
                let too_large = || {
                    ReadError::Archive(io::Error::other(format!(
                        "the archive's JSON files are larger than {} MiB",
                        METADATA_LIMIT >> 20
                    )))
                };
                if entry.size() > metadata_left {
                    return Err(too_large());
                }
                let mut bytes = Vec::new();
                (&mut entry)
                    .take(metadata_left + 1)
                    .read_to_end(&mut bytes)?;
                metadata_left = metadata_left
                    .checked_sub(bytes.len() as u64)
                    .ok_or_else(too_large)?;
                Member::Metadata(bytes.into())
            } else {
                let (digest, size, compressed) = compress(&mut entry, compression)?;
                put(&Blob {
                    digest: digest.clone(),
                    size,
                    data: match &compressed {
                        SpooledData::InMemory(cursor) => BlobData::Bytes(cursor.get_ref()),
                        SpooledData::OnDisk(file) => BlobData::File(file),
                    },
                })
                .map_err(ReadError::Blob)?;
                Member::Layer { digest, size }
            };
            members.files.insert(name, member);
        }
        // Read to the end, so a streaming executable is not cut off after the end-of-archive marker.
        io::copy(&mut archive.into_inner(), &mut io::sink())?;
        Ok(members)
    }

    /// The file an archive path names, following symbolic links.
    fn get(&self, name: &str) -> Result<&Member, String> {
        let current = normalize(Path::new(name))
            .filter(|_| {
                Path::new(name)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
            })
            .ok_or_else(|| format!("the archive refers to {name:?} outside itself"))?;
        let mut current = current.as_str();
        for _ in 0..16 {
            if let Some(member) = self.files.get(current) {
                return Ok(member);
            }
            match self.links.get(current) {
                Some(target) => current = target,
                None => break,
            }
        }
        Err(format!("the archive has no file {name:?}"))
    }

    fn metadata(&self, name: &str) -> Result<&[u8], String> {
        match self.get(name)? {
            Member::Metadata(bytes) => Ok(bytes),
            Member::Layer { .. } => Err(format!(
                "{name} must be a .json file, as in the archives dockerTools writes"
            )),
        }
    }
}

/// An archive path without `.` components and with `..` resolved, or `None` if it leaves the
/// archive.
fn normalize(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl Image {
    /// Read a fetched image and hand on each of its blobs: an executable that streams a Docker
    /// image archive, such as the output of `dockerTools.streamLayeredImage`, or the archive
    /// itself, gzip- or zstd-compressed or not. The Nix store keeps only the executable bit of a
    /// file's mode, so that bit, not the content, tells the two apart. The layers are handed on
    /// while the archive is read, before the declared platform can be checked against the
    /// config; the config comes last.
    pub fn convert(
        platform: &str,
        path: impl AsRef<Path>,
        compression: Compression,
        put: &mut dyn FnMut(&Blob<'_>) -> Result<(), String>,
    ) -> Result<Self, String> {
        let path = path.as_ref();
        let expected = Platform::parse(platform)?;
        let failed = |error: &dyn fmt::Display| {
            format!(
                "could not read image {platform} ({}): {error}",
                path.display()
            )
        };
        let read_failed = |error: ReadError| match error {
            ReadError::Archive(error) => failed(&error),
            ReadError::Blob(error) => error,
        };
        let metadata = std::fs::metadata(path).map_err(|error| failed(&error))?;
        if !metadata.is_file() {
            return Err(failed(
                &"not a file; expected an image archive or an executable that streams one",
            ));
        }
        let mut read = |reader: &mut dyn Read| Members::read(reader, compression, put);

        let members = if metadata.permissions().mode() & 0o111 != 0 {
            let mut child = Command::new(path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|error| failed(&format!("could not run the image executable: {error}")))?;
            let mut stdout = child.stdout.take().expect("piped stdout");
            let members = read(&mut stdout);
            drop(stdout);
            let status = child.wait().map_err(|error| failed(&error))?;
            let members = members.map_err(read_failed)?;
            if !status.success() {
                return Err(failed(&format!(
                    "the image executable exited with {status}"
                )));
            }
            members
        } else {
            let mut file = File::open(path).map_err(|error| failed(&error))?;
            let mut magic = [0; 4];
            let count = read_prefix(&mut file, &mut magic).map_err(|error| failed(&error))?;
            let magic = &magic[..count];
            file.rewind().map_err(|error| failed(&error))?;
            if magic.starts_with(&[0x1f, 0x8b]) {
                read(&mut flate2::read::GzDecoder::new(BufReader::new(file)))
            } else if magic == [0x28, 0xb5, 0x2f, 0xfd] {
                match zstd::Decoder::new(file) {
                    Ok(mut decoder) => read(&mut decoder),
                    Err(error) => Err(ReadError::Archive(error)),
                }
            } else {
                read(&mut BufReader::new(file))
            }
            .map_err(read_failed)?
        };

        Self::from_members(&expected, path, compression, &members, put)
            .map_err(|error| failed(&error))
    }

    fn from_members(
        expected: &Platform,
        path: &Path,
        compression: Compression,
        members: &Members,
        put: &mut dyn FnMut(&Blob<'_>) -> Result<(), String>,
    ) -> Result<Self, String> {
        let entries: Vec<ArchiveEntry> = serde_json::from_slice(
            members
                .metadata("manifest.json")
                .map_err(|error| format!("the archive has no readable manifest.json: {error}"))?,
        )
        .map_err(|error| format!("invalid manifest.json: {error}"))?;
        let [entry] = entries.as_slice() else {
            return Err(format!(
                "the archive must hold exactly one image, found {}",
                entries.len()
            ));
        };
        let config = members.metadata(&entry.config).map_err(|error| {
            format!("could not read the image config {}: {error}", entry.config)
        })?;
        let image_config: ImageConfig = serde_json::from_slice(config)
            .map_err(|error| format!("invalid image config {}: {error}", entry.config))?;
        let platform = Platform {
            architecture: image_config.architecture,
            os: image_config.os,
            variant: image_config.variant.filter(|variant| !variant.is_empty()),
        };
        if platform != *expected {
            return Err(format!(
                "the image is built for {platform}, but is declared as {expected}"
            ));
        }

        let descriptor = |media_type: &str, digest: &str, size: u64| Descriptor {
            media_type: media_type.into(),
            digest: digest.into(),
            size,
            platform: None,
            annotations: BTreeMap::new(),
        };
        let layers = entry
            .layers
            .iter()
            .map(|layer| match members.get(layer)? {
                Member::Layer { digest, size } => {
                    Ok(descriptor(compression.layer_media_type(), digest, *size))
                }
                Member::Metadata(_) => Err(format!(
                    "layer {layer} must not be a .json file, as in the archives dockerTools writes"
                )),
            })
            .collect::<Result<Vec<_>, String>>()?;
        let config_digest = sha256_digest(config);
        put(&Blob {
            digest: config_digest.clone(),
            size: config.len() as u64,
            data: BlobData::Bytes(config),
        })?;
        let manifest = serde_json::to_vec(&ImageManifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE,
            config: descriptor(CONFIG_MEDIA_TYPE, &config_digest, config.len() as u64),
            layers,
        })
        .expect("serialize an image manifest");
        let manifest_digest = sha256_digest(&manifest);
        Ok(Self {
            platform,
            store_path: path.display().to_string(),
            manifest,
            manifest_digest,
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
            annotations: BTreeMap::from([(STORE_PATH_ANNOTATION.into(), self.store_path.clone())]),
        }
    }
}

impl Compression {
    fn layer_media_type(self) -> &'static str {
        match self {
            Self::Gzip { .. } => GZIP_LAYER_MEDIA_TYPE,
            Self::Zstd { .. } => ZSTD_LAYER_MEDIA_TYPE,
        }
    }
}

/// Hashes and counts what is written through it.
struct Hashing<T> {
    inner: T,
    hasher: Sha256,
    size: u64,
}

impl<T: io::Write> io::Write for Hashing<T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Compress a stream, in memory up to `SPOOL_LIMIT` and in a temporary file beyond; the digest
/// and size of what was written, and the compressed bytes.
fn compress(
    mut source: impl Read,
    compression: Compression,
) -> io::Result<(String, u64, SpooledData)> {
    let target = Hashing {
        inner: io::BufWriter::new(tempfile::SpooledTempFile::new(SPOOL_LIMIT)),
        hasher: Sha256::new(),
        size: 0,
    };
    let target = match compression {
        Compression::Gzip { level } => {
            let mut encoder =
                flate2::write::GzEncoder::new(target, flate2::Compression::new(level));
            io::copy(&mut source, &mut encoder)?;
            encoder.finish()?
        }
        Compression::Zstd { level } => {
            let mut encoder = zstd::Encoder::new(target, level)?;
            io::copy(&mut source, &mut encoder)?;
            encoder.finish()?
        }
    };
    let Hashing {
        inner,
        hasher,
        size,
    } = target;
    let spooled = inner.into_inner().map_err(io::IntoInnerError::into_error)?;
    Ok((hex_digest(&hasher.finalize()), size, spooled.into_inner()))
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

/// The image index for a release: one manifest per platform, annotated with the version.
#[must_use]
pub fn image_index(images: &[Image], version: &Version) -> (Vec<u8>, String) {
    let mut manifests: Vec<_> = images.iter().map(Image::descriptor).collect();
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
    pub fn from_environment(
        username: &crate::declaration::Username,
        password: &str,
    ) -> Result<Self, PublicationError> {
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
            username: match username {
                crate::declaration::Username::Secret(name) => read(name)?,
                crate::declaration::Username::Value(username) => username.clone(),
            },
            password: read(password)?,
        })
    }
}

enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    File(&'a File, u64),
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
                Body::File(file, size) => {
                    // A retry after authenticating sends the file again from its start.
                    let mut file = *file;
                    file.rewind().map_err(|error| {
                        CallError::Transport(format!("could not read a compressed layer: {error}"))
                    })?;
                    request
                        .set("Content-Length", &size.to_string())
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
    pub fn upload_blob(&self, blob: &Blob<'_>) -> Result<(), PublicationError> {
        let failure = match self.start_upload() {
            Ok(mut location) => {
                location
                    .query_pairs_mut()
                    .append_pair("digest", &blob.digest);
                match self.call(
                    "PUT",
                    &location,
                    &[("Content-Type", "application/octet-stream")],
                    &match blob.data {
                        BlobData::File(path) => Body::File(path, blob.size),
                        BlobData::Bytes(bytes) => Body::Bytes(bytes),
                    },
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
    /// The image of each platform, by store path; read only while it is pushed.
    platforms: BTreeMap<Platform, PathBuf>,
    compression: Compression,
}

/// An index the registry already holds for this release, which further tags point to.
#[derive(Debug, Clone)]
struct RemoteIndex {
    digest: String,
    bytes: Vec<u8>,
}

impl OciPublisher {
    /// Build the publisher from its declaration; its images must already be fetched.
    pub fn from_declaration(
        declaration: &crate::declaration::OciPublisher,
    ) -> Result<Self, PublicationError> {
        let credentials =
            Credentials::from_environment(&declaration.username, &declaration.password.secret)?;
        Self::new(
            RegistryClient::new(&declaration.repository, Some(credentials))?,
            &declaration.repository,
            declaration.tags.clone(),
            declaration.aliases.clone(),
            declaration
                .platforms
                .iter()
                .map(|(platform, path)| (platform.clone(), PathBuf::from(path))),
            declaration.compression,
        )
    }

    pub fn new(
        registry: RegistryClient,
        repository: &str,
        tags: Vec<String>,
        aliases: Vec<String>,
        platforms: impl IntoIterator<Item = (String, PathBuf)>,
        compression: Compression,
    ) -> Result<Self, PublicationError> {
        let platforms = platforms
            .into_iter()
            .map(|(platform, path)| Ok((Platform::parse(&platform)?, path)))
            .collect::<Result<_, String>>()
            .map_err(PublicationError::new)?;
        Ok(Self {
            registry,
            repository: repository.into(),
            tags,
            aliases,
            platforms,
            compression,
        })
    }

    fn reference(&self, tag: &str) -> String {
        format!("{}:{tag}", self.repository)
    }

    /// Whether an index is this release's: the same version, and the same store path for every
    /// platform. Compressed bytes may differ between runs, so digests are not compared.
    fn is_release_index(&self, manifest: &[u8], version: &Version) -> bool {
        #[derive(Deserialize)]
        struct Entry {
            platform: Option<Platform>,
            #[serde(default)]
            annotations: BTreeMap<String, String>,
        }
        #[derive(Deserialize)]
        struct Index {
            manifests: Vec<Entry>,
        }
        if annotated_version(manifest).as_ref() != Some(version) {
            return false;
        }
        let Ok(index) = serde_json::from_slice::<Index>(manifest) else {
            return false;
        };
        let mut images = BTreeMap::new();
        for entry in index.manifests {
            let (Some(platform), Some(path)) =
                (entry.platform, entry.annotations.get(STORE_PATH_ANNOTATION))
            else {
                return false;
            };
            // A second manifest for a platform could be the one a client picks.
            if images.insert(platform, PathBuf::from(path)).is_some() {
                return false;
            }
        }
        images == self.platforms
    }

    fn inspect(&self, release: &Release) -> Result<Inspection, PublicationError> {
        let tags = render_tags(&self.tags, &self.aliases, &release.version)
            .map_err(PublicationError::new)?;
        let mut existing = None;
        let mut states = Vec::new();
        for tag in tags {
            let (state, index) = self.tag_state(&tag, release)?;
            existing = existing.or(index);
            states.push((tag, state));
        }
        Ok(Inspection {
            existing,
            tags: states,
        })
    }

    /// The state of a tag, with the index it points to if that is already this release's.
    fn tag_state(
        &self,
        tag: &OciTag,
        release: &Release,
    ) -> Result<(TagState, Option<RemoteIndex>), PublicationError> {
        let matching = |digest: String, bytes: Vec<u8>| {
            (TagState::Matching, Some(RemoteIndex { digest, bytes }))
        };
        match tag {
            OciTag::Version(name) => Ok(match self.registry.manifest(name)? {
                None => (TagState::Missing, None),
                Some((actual, manifest)) if self.is_release_index(&manifest, &release.version) => {
                    matching(actual, manifest)
                }
                Some((actual, _)) => (
                    TagState::Conflicting(format!(
                        "{} points to {actual}, which is not the image index of version {} with these images",
                        self.reference(name),
                        release.version
                    )),
                    None,
                ),
            }),
            OciTag::Alias(name) => {
                let prerelease = !release.version.pre.is_empty();
                // Aliases are best effort: an alias that cannot be read is reported, not fatal.
                let current = match self.registry.manifest(name) {
                    Ok(current) => current,
                    Err(error) => return Ok((TagState::Unknown(error.to_string()), None)),
                };
                let Some((actual, manifest)) = current else {
                    return Ok((
                        if prerelease {
                            TagState::Kept("pre-release".into())
                        } else {
                            TagState::Missing
                        },
                        None,
                    ));
                };
                if self.is_release_index(&manifest, &release.version) {
                    return Ok(matching(actual, manifest));
                }
                if prerelease {
                    return Ok((TagState::Kept("pre-release".into()), None));
                }
                let state = match annotated_version(&manifest) {
                    None => TagState::Kept(format!("points to {actual} without a version")),
                    Some(current) if current > release.version => {
                        TagState::Kept(format!("{current} is higher"))
                    }
                    Some(current) if current == release.version => TagState::Conflicting(format!(
                        "{} points to {actual}, an index of version {current} with other images",
                        self.reference(name)
                    )),
                    Some(_) => TagState::Behind,
                };
                Ok((state, None))
            }
        }
    }

    /// Read an image and push every blob and its manifest that the registry does not have yet.
    /// Each layer is pushed as soon as it is compressed; the image's platform is checked before
    /// its manifest is written.
    fn push_image(
        &self,
        platform: &Platform,
        path: &Path,
    ) -> Result<(Image, AssetStep), PublicationError> {
        let mut pushed = false;
        let image = Image::convert(&platform.to_string(), path, self.compression, &mut |blob| {
            if !self
                .registry
                .blob_exists(&blob.digest)
                .map_err(|error| error.to_string())?
            {
                self.registry
                    .upload_blob(blob)
                    .map_err(|error| error.to_string())?;
                pushed = true;
            }
            Ok(())
        })
        .map_err(PublicationError::new)?;
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
        let step = if pushed {
            AssetStep::Uploaded
        } else {
            AssetStep::AlreadyUploaded
        };
        Ok((image, step))
    }

    /// Point an alias at the index if it still needs to move; the error is the warning text.
    fn move_alias(
        &self,
        tag: &OciTag,
        release: &Release,
        index: &RemoteIndex,
    ) -> Result<(), String> {
        match self
            .tag_state(tag, release)
            .map_err(|error| error.to_string())?
            .0
        {
            TagState::Missing | TagState::Behind => self
                .registry
                .put_manifest(tag.name(), INDEX_MEDIA_TYPE, &index.bytes, &index.digest)
                .map_err(|error| error.to_string()),
            TagState::Unknown(reason) | TagState::Conflicting(reason) => Err(reason),
            TagState::Matching | TagState::Kept(_) => Ok(()),
        }
    }

    fn steps(&self, inspection: &Inspection, images: Vec<AssetStep>) -> ArtifactSteps {
        let mut steps: ArtifactSteps = self
            .platforms
            .keys()
            .zip(images)
            .map(|(platform, step)| ArtifactStep {
                kind: ArtifactKind::Image,
                name: platform.to_string(),
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

    /// The release's index: the one a tag already points to, or a new one built from the images,
    /// which are pushed first.
    fn release_index(
        &self,
        inspection: &Inspection,
        release: &Release,
    ) -> Result<(RemoteIndex, Vec<AssetStep>), PublicationError> {
        if let Some(index) = &inspection.existing {
            return Ok((
                index.clone(),
                vec![AssetStep::AlreadyUploaded; self.platforms.len()],
            ));
        }
        let (images, steps): (Vec<_>, Vec<_>) = self
            .platforms
            .iter()
            .map(|(platform, path)| self.push_image(platform, path))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        let (bytes, digest) = image_index(&images, &release.version);
        if self.registry.manifest_digest(&digest)?.is_none() {
            self.registry
                .put_manifest(&digest, INDEX_MEDIA_TYPE, &bytes, &digest)?;
        }
        Ok((RemoteIndex { digest, bytes }, steps))
    }
}

#[derive(Debug)]
struct Inspection {
    /// An index of this release that a tag already points to.
    existing: Option<RemoteIndex>,
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
        let (index, images) = self.release_index(&inspection, release)?;

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
                if let Err(reason) = self.move_alias(tag, release, &index) {
                    step.step = AssetStep::Warning(reason);
                } else if let Ok((state, _)) = self.tag_state(tag, release) {
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
            match self.tag_state(tag, release)?.0 {
                TagState::Missing | TagState::Behind => {
                    self.registry.put_manifest(
                        tag.name(),
                        INDEX_MEDIA_TYPE,
                        &index.bytes,
                        &index.digest,
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
        let images = vec![AssetStep::AlreadyUploaded; self.platforms.len()];
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
    fn spellings_of_one_repository_share_an_identity() {
        let identity = |repository| RepositoryReference::parse(repository).unwrap().identity();
        for (left, right) in [
            ("docker.io/alpine", "docker.io/library/alpine"),
            ("docker.io/owner/image", "registry-1.docker.io/owner/image"),
            ("docker.io/owner/image", "index.docker.io/owner/image"),
            ("GHCR.IO/owner/image", "ghcr.io/owner/image"),
            ("ghcr.io:443/owner/image", "ghcr.io/owner/image"),
            ("localhost:80/image", "localhost/image"),
        ] {
            assert_eq!(identity(left), identity(right), "{left} and {right}");
        }
        assert_ne!(
            identity("ghcr.io:5000/owner/image"),
            identity("ghcr.io/owner/image")
        );
        assert_ne!(
            identity("docker.io/alpine"),
            identity("docker.io/owner/alpine")
        );
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
        assert!(validate(&["{version}"], &["{major}", "{minor}"]).is_ok());
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
