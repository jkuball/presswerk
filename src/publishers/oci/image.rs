//! The OCI image publisher: pushes images built with Nix to an OCI registry under the release's tags.
//!
//! Images come as Docker image archives, the output of `dockerTools.buildImage` and
//! `buildLayeredImage` (a tarball, possibly compressed) or `streamLayeredImage` (an executable
//! that writes the archive). Presswerk compresses their layers with gzip or zstd, as configured,
//! and writes the OCI manifests itself. Each manifest in the index records the Nix store path of
//! its image, so a retry recognizes the release's index by its store paths, whatever bytes the
//! compressor produced.

use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::shared::secrets::Credential;
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, WarningLevel, shared,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::{Arc, mpsc};
use std::thread;
use tempfile::SpooledData;

use super::*;

/// The image of one platform.
pub const IMAGE: ArtifactKind = ArtifactKind::new("image", "pushed");
/// A tag that follows the highest final version, such as `latest`.
pub const ALIAS: ArtifactKind = ArtifactKind::new("alias", "set");

/// `publishers.oci-image.<name>`: container images pushed to an OCI registry under tags rendered
/// from the version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciImageDeclaration {
    /// Registry host and repository path, such as `ghcr.io/owner/project`.
    pub repository: String,
    /// The image for each platform, such as `linux/amd64`, each a path in the Nix store.
    pub platforms: BTreeMap<String, String>,
    /// Tag templates of the release; each contains `{version}`.
    pub tags: Vec<String>,
    /// Alias templates, such as `latest` or `{major}.{minor}`, that move to a higher version.
    /// Missing in declarations written before the option existed.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Whether anyone may pull the images; the previous-release check then reads anonymously.
    /// Missing in declarations written before the option existed.
    #[serde(default)]
    pub public: bool,
    /// How layers are compressed for the registry.
    /// Missing in declarations written before the option existed.
    #[serde(default)]
    pub compression: Compression,
    pub username: Credential,
    pub password: Credential,
}

/// The compression of pushed image layers, with its level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Compression {
    Gzip { level: u32 },
    Zstd { level: i32 },
}

impl Default for Compression {
    fn default() -> Self {
        Self::Gzip { level: 6 }
    }
}

impl OciImageDeclaration {
    fn registry(&self) -> Registry<'_> {
        Registry {
            repository: &self.repository,
            public: self.public,
            username: &self.username,
            password: &self.password,
        }
    }

    fn validate(&self) -> Result<(), String> {
        RepositoryReference::parse(&self.repository)?;
        if self.platforms.is_empty() {
            return Err("an OCI image publisher needs at least one image".into());
        }
        for (platform, path) in &self.platforms {
            Platform::parse(platform)?;
            if !path.starts_with('/') {
                return Err(format!(
                    "image {platform} must be an absolute store path, not {path:?}"
                ));
            }
        }
        validate_tag_templates(&self.tags, &self.aliases)?;
        match self.compression {
            Compression::Gzip { level } if !(1..=9).contains(&level) => {
                return Err(format!("gzip level {level} must be between 1 and 9"));
            }
            Compression::Zstd { level } if !(1..=22).contains(&level) => {
                return Err(format!("zstd level {level} must be between 1 and 22"));
            }
            _ => {}
        }
        self.registry().validate()
    }
}

/// The `oci-image` publisher type.
#[derive(Debug)]
pub struct OciImage {
    declaration: OciImageDeclaration,
    /// Built on first use, once the images are fetched and the credentials are known.
    runner: OnceLock<OciImagePublisher>,
}

impl OciImage {
    #[must_use]
    pub fn declaration(&self) -> &OciImageDeclaration {
        &self.declaration
    }

    fn runner(&self, ctx: &Context) -> Result<&OciImagePublisher, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let declaration = &self.declaration;
        let runner = OciImagePublisher::new(
            declaration
                .registry()
                .writing_client(&ctx.session.secrets)?,
            &declaration.repository,
            declaration.tags.clone(),
            declaration.aliases.clone(),
            declaration
                .platforms
                .iter()
                .map(|(platform, path)| (platform.clone(), PathBuf::from(path))),
            declaration.compression,
        )?
        .with_trusted_predecessor(ctx.session.trusted_predecessor.clone());
        Ok(self.runner.get_or_init(|| runner))
    }
}

impl PublisherType for OciImage {
    const NAME: &'static str = "oci-image";
    const ALIASES: &'static [&'static str] = &["oci"];

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: OciImageDeclaration = shared::decode(declaration)?;
        declaration.validate().map_err(DeclarationError::Invalid)?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

/// Describes the images and tags that a release of `version` pushes.
fn describe(
    repository: &str,
    platforms: impl Iterator<Item = String>,
    tags: &[String],
    aliases: &[String],
    version: &Version,
) -> Result<Description, String> {
    // Store paths are left out: the release commit's images get their own paths.
    let images = platforms.map(|platform| ("image".to_owned(), platform));
    let tags = render_tags(tags, aliases, version)?.into_iter().map(|tag| {
        let kind = match tag {
            OciTag::Version(_) => "tag",
            OciTag::Alias(_) => "alias",
        };
        (kind.to_owned(), tag.name().to_owned())
    });
    Ok(Description {
        target: repository.into(),
        items: images.chain(tags).collect(),
    })
}

/// Whether every version tag of `release` points to its image index, as `manifest` reads the
/// manifest a tag points to. Aliases do not count: they are best effort and may point to another
/// release. A tag that cannot be read, such as when the registry denies access or is unreachable,
/// makes the release `Unknown`.
pub fn completeness_with(
    repository: &str,
    tags: &[String],
    aliases: &[String],
    release: &Release,
    mut manifest: impl FnMut(&str) -> Result<Option<Vec<u8>>, PublicationError>,
) -> Result<Completeness, PublicationError> {
    let version = &release.version;
    for tag in render_tags(tags, aliases, version).map_err(PublicationError::new)? {
        let OciTag::Version(tag) = tag else {
            continue;
        };
        let read = match manifest(&tag) {
            Ok(read) => read,
            Err(error) => return Ok(Completeness::Unknown(error.to_string())),
        };
        let found = match read {
            None => "the tag is missing".to_owned(),
            Some(manifest) => match annotated_version(&manifest) {
                Some(found) if found == *version => continue,
                Some(found) => format!("the tag points to version {found}"),
                None => {
                    "the tag does not point to an image index with a version annotation".to_owned()
                }
            },
        };
        return Ok(Completeness::Incomplete {
            target: format!("OCI image release {repository}:{tag}"),
            reason: format!("{found}, expected version {version}"),
        });
    }
    Ok(Completeness::Complete)
}

impl Publisher for OciImage {
    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        vec![
            ("username", &self.declaration.username),
            ("password", &self.declaration.password),
        ]
    }

    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        let declaration = &self.declaration;
        describe(
            &declaration.repository,
            declaration.platforms.keys().cloned(),
            &declaration.tags,
            &declaration.aliases,
            version,
        )
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        self.declaration
            .platforms
            .iter()
            .map(|(platform, path)| (format!("image {platform}"), path.clone()))
            .collect()
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
            &declaration.tags,
            &declaration.aliases,
            release,
            |tag| Ok(registry.manifest(tag)?.map(|(_, manifest)| manifest)),
        )
    }

    /// Reads the aliases of the release, so an alias another tool pushed does not stay behind
    /// unnoticed.
    fn preview(&self, release: &Release, ctx: &Context) {
        let declaration = &self.declaration;
        if declaration.aliases.is_empty() {
            return;
        }
        let secrets = &ctx.session.secrets;
        let variables =
            missing_credentials(declaration, |name| secrets.env(name).map(str::to_owned));
        // Without credentials, an anonymous read may still succeed, such as for a repository
        // that is public without being marked so.
        let registry = if variables.is_empty() {
            declaration.registry().reading_client(secrets)
        } else {
            let verb = if variables.len() == 1 { "is" } else { "are" };
            ctx.warn(
                WarningLevel::Warning,
                format!(
                    "{} is not marked public and {} {verb} not set; checking its aliases anonymously, which will probably fail",
                    ctx.address,
                    variables.join("/")
                ),
            );
            RegistryClient::new(&declaration.repository, None)
        };
        let found = registry.and_then(|registry| {
            foreign_aliases(
                &registry,
                &declaration.repository,
                &declaration.tags,
                &declaration.aliases,
                &release.version,
                ctx.session.trusted_predecessor.as_ref(),
            )
        });
        match found {
            Ok(found) => {
                for alias in found {
                    match &alias.predecessor_tag {
                        Some(tag) => ctx.warn(
                            WarningLevel::Note,
                            format!(
                                "alias {} points to the trusted predecessor's tag {tag}; publish will take it over",
                                alias.reference
                            ),
                        ),
                        None => ctx.warn(
                            WarningLevel::Warning,
                            format!(
                                "alias {} points to {} without a version annotation, so another tool pushed it; publish will keep it",
                                alias.reference, alias.digest
                            ),
                        ),
                    }
                }
            }
            Err(error) => ctx.warn(
                WarningLevel::Warning,
                format!("could not check the aliases of {}: {error}", ctx.address),
            ),
        }
    }

    fn state(&self, release: &Release, ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.runner(ctx)?.state(release, ctx)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        self.runner(ctx)?.publish(release, ctx)
    }

    fn complete_steps(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        self.runner(ctx)?.complete_steps(release, ctx)
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

/// One rendered tag of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OciTag {
    /// A tag of this version, such as `1.2.0`; it is never moved once pushed.
    Version(String),
    /// An alias, such as `latest` or `1.2`; it moves to a higher final version.
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
            "an OCI image publisher needs at least one tag, so every release has a tag of its own"
                .into(),
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
    shared::template::render(template, version)
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

/// The largest compressed layer kept in memory during ordinary conversion.
const SPOOL_LIMIT: usize = 64 << 20;

/// The per-layer in-memory limit while uploads are pipelined. Six workers plus the producer can
/// retain seven compressed blobs, so larger layers spill to disk to keep the total bound modest.
const PIPELINED_SPOOL_LIMIT: usize = 16 << 20;

/// Layers uploaded concurrently within one image. Each worker has a rendezvous channel, so no
/// additional compressed blobs queue behind the workers.
const BLOB_UPLOAD_LIMIT: usize = 6;

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

struct OwnedBlob {
    digest: String,
    size: u64,
    data: SpooledData,
}

impl OwnedBlob {
    fn borrowed(&self) -> Blob<'_> {
        Blob {
            digest: self.digest.clone(),
            size: self.size,
            data: match &self.data {
                SpooledData::InMemory(cursor) => BlobData::Bytes(cursor.get_ref()),
                SpooledData::OnDisk(file) => BlobData::File(file),
            },
        }
    }
}

fn record_upload_result(
    (sequence, result): (usize, Result<bool, PublicationError>),
    pending: &mut usize,
    pushed: &mut bool,
    error: &mut Option<(usize, PublicationError)>,
) {
    *pending -= 1;
    match result {
        Ok(uploaded) => *pushed |= uploaded,
        Err(upload_error)
            if error
                .as_ref()
                .is_none_or(|(failed_sequence, _)| sequence < *failed_sequence) =>
        {
            *error = Some((sequence, upload_error));
        }
        Err(_) => {}
    }
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Archive(error)
    }
}

impl Members {
    /// Read an archive, handing every layer on as soon as it is compressed. Which files are layers
    /// is known only from `manifest.json`, which may come last, so JSON files are kept and every
    /// other file is treated as a layer; a Docker image archive holds nothing else.
    fn read(
        reader: impl Read,
        compression: Compression,
        spool_limit: usize,
        put: &mut dyn FnMut(OwnedBlob) -> Result<(), String>,
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
                let (digest, size, data) = compress(&mut entry, compression, spool_limit)?;
                put(OwnedBlob {
                    digest: digest.clone(),
                    size,
                    data,
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
        Self::convert_owned(platform, path, compression, SPOOL_LIMIT, &mut |blob| {
            put(&blob.borrowed())
        })
    }

    fn convert_owned(
        platform: &str,
        path: impl AsRef<Path>,
        compression: Compression,
        spool_limit: usize,
        put: &mut dyn FnMut(OwnedBlob) -> Result<(), String>,
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
        let mut read = |reader: &mut dyn Read| Members::read(reader, compression, spool_limit, put);

        let members = if metadata.permissions().mode() & 0o111 != 0 {
            let mut child = image_command(path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| failed(&format!("could not run the image executable: {error}")))?;
            // The executable inherits the environment, so its diagnostics could show a secret.
            // They are drained while stdout is read, so a noisy executable cannot block.
            let stderr = child.stderr.take().expect("piped stderr");
            let diagnostics = thread::spawn(move || {
                forward_diagnostics(BufReader::new(stderr), &mut io::stderr())
            });
            let mut stdout = child.stdout.take().expect("piped stdout");
            let members = read(&mut stdout);
            drop(stdout);
            let status = child.wait().map_err(|error| failed(&error))?;
            let _ = diagnostics.join();
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
        put: &mut dyn FnMut(OwnedBlob) -> Result<(), String>,
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

        let descriptor = Descriptor::new;
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
        put(OwnedBlob {
            digest: config_digest.clone(),
            size: config.len() as u64,
            data: SpooledData::InMemory(io::Cursor::new(config.to_vec())),
        })?;
        let manifest = serde_json::to_vec(&Manifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE.into(),
            config: descriptor(CONFIG_MEDIA_TYPE, &config_digest, config.len() as u64),
            layers,
            annotations: BTreeMap::new(),
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
            annotations: BTreeMap::from([(ORIGIN_ANNOTATION.into(), self.store_path.clone())]),
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

/// Compress a stream, in memory up to `spool_limit` and in a temporary file beyond; the digest and
/// size of what was written, and the compressed bytes.
fn compress(
    mut source: impl Read,
    compression: Compression,
    spool_limit: usize,
) -> io::Result<(String, u64, SpooledData)> {
    let target = Hashing {
        inner: io::BufWriter::new(tempfile::SpooledTempFile::new(spool_limit)),
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

/// The command that runs an image executable. It never needs a publisher's secret; without them,
/// its diagnostics cannot show one, not even a value that spans several lines, which line-by-line
/// redaction would miss.
fn image_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    for name in crate::publishers::shared::secrets::secret_variables() {
        command.env_remove(name);
    }
    command
}

/// Copies what an image executable prints on stderr to `sink`, line by line, with every secret
/// value replaced.
fn forward_diagnostics(reader: impl io::BufRead, sink: &mut dyn io::Write) {
    for line in reader.split(b'\n') {
        let Ok(line) = line else {
            return;
        };
        let line = crate::publishers::shared::secrets::redact(&String::from_utf8_lossy(&line));
        let _ = writeln!(sink, "{line}");
    }
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

/// What a registry holds under one tag, compared with what this release pushes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagState {
    Missing,
    Matching,
    /// An alias that points to a lower version and moves to this one.
    Behind,
    /// An alias that is left alone, with the reason.
    Kept(String),
    /// An alias without a version annotation that points to the trusted predecessor's release, whose tool
    /// pushed it; it moves to this release once, named by the predecessor's version tag.
    TakeOver(String),
    /// An alias whose state could not be read; it is left alone and repaired by a later run.
    Unknown(String),
    Conflicting(String),
}

/// Pushes one image index per release and points every tag at it.
#[derive(Debug)]
pub struct OciImagePublisher {
    registry: RegistryClient,
    repository: String,
    tags: Vec<String>,
    aliases: Vec<String>,
    /// The image of each platform, by store path; read only while it is pushed.
    platforms: BTreeMap<Platform, PathBuf>,
    compression: Compression,
    /// The version of a trusted predecessor released by another tool, whose aliases this release
    /// takes over.
    predecessor: Option<Version>,
}

/// An index the registry already holds for this release, which further tags point to.
#[derive(Debug, Clone)]
struct RemoteIndex {
    digest: String,
    bytes: Vec<u8>,
}

impl OciImagePublisher {
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
            predecessor: None,
        })
    }

    /// Take over the aliases that another tool pushed for the trusted predecessor `version`, as
    /// recorded in the release commit, when they still point to its release.
    #[must_use]
    pub fn with_trusted_predecessor(mut self, version: Option<Version>) -> Self {
        self.predecessor = version;
        self
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
                (entry.platform, entry.annotations.get(ORIGIN_ANNOTATION))
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
                    None => match predecessor_tag(
                        &self.registry,
                        &self.tags,
                        self.predecessor.as_ref(),
                        &actual,
                    ) {
                        Ok(Some(tag)) => TagState::TakeOver(tag),
                        Ok(None) => TagState::Kept(format!(
                            "points to {actual} without a version annotation"
                        )),
                        Err(error) => TagState::Unknown(error.to_string()),
                    },
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
    /// Layers upload concurrently while following layers are read and compressed; the image's
    /// platform is checked before its manifest is written.
    fn push_image(
        &self,
        platform: &Platform,
        path: &Path,
    ) -> Result<(Image, AssetStep), PublicationError> {
        let (image, mut pushed) = thread::scope(|scope| {
            let (available, ready) = mpsc::channel();
            let (results, completed) = mpsc::channel();
            let mut workers = Vec::with_capacity(BLOB_UPLOAD_LIMIT);
            for worker in 0..BLOB_UPLOAD_LIMIT {
                let (blobs, uploads) = mpsc::sync_channel::<(usize, OwnedBlob)>(0);
                workers.push(blobs);
                let available = available.clone();
                let results = results.clone();
                scope.spawn(move || {
                    while available.send(worker).is_ok() {
                        let Ok((sequence, blob)) = uploads.recv() else {
                            break;
                        };
                        if results
                            .send((sequence, self.registry.push_blob(&blob.borrowed())))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
            drop(available);
            drop(results);

            let mut pending = 0;
            let mut pushed = false;
            let mut upload_error: Option<(usize, PublicationError)> = None;
            let mut sequence = 0;
            let image = Image::convert_owned(
                &platform.to_string(),
                path,
                self.compression,
                PIPELINED_SPOOL_LIMIT,
                &mut |blob| {
                    while let Ok(result) = completed.try_recv() {
                        record_upload_result(result, &mut pending, &mut pushed, &mut upload_error);
                    }
                    if let Some((_, error)) = &upload_error {
                        return Err(error.to_string());
                    }
                    let worker = ready
                        .recv()
                        .map_err(|_| "a blob upload worker stopped unexpectedly".to_owned())?;
                    while let Ok(result) = completed.try_recv() {
                        record_upload_result(result, &mut pending, &mut pushed, &mut upload_error);
                    }
                    if let Some((_, error)) = &upload_error {
                        return Err(error.to_string());
                    }
                    workers[worker]
                        .send((sequence, blob))
                        .map_err(|_| "a blob upload worker stopped unexpectedly".to_owned())?;
                    sequence += 1;
                    pending += 1;
                    Ok(())
                },
            );
            drop(workers);
            while pending > 0 {
                let result = completed.recv().map_err(|_| {
                    PublicationError::new("a blob upload worker stopped unexpectedly")
                })?;
                record_upload_result(result, &mut pending, &mut pushed, &mut upload_error);
            }
            if let Some((_, error)) = upload_error {
                return Err(error);
            }
            Ok((image.map_err(PublicationError::new)?, pushed))
        })?;
        pushed |= self.registry.push_manifest(
            MANIFEST_MEDIA_TYPE,
            &image.manifest,
            &image.manifest_digest,
        )?;
        let step = if pushed {
            AssetStep::Uploaded
        } else {
            AssetStep::AlreadyUploaded
        };
        Ok((image, step))
    }

    /// Point an alias at the index if it still needs to move, and tell whether it was taken over
    /// from the trusted predecessor; the error is the warning text.
    fn move_alias(
        &self,
        tag: &OciTag,
        release: &Release,
        index: &RemoteIndex,
    ) -> Result<bool, String> {
        let state = self
            .tag_state(tag, release)
            .map_err(|error| error.to_string())?
            .0;
        match state {
            TagState::Missing | TagState::Behind | TagState::TakeOver(_) => self
                .registry
                .put_manifest(tag.name(), INDEX_MEDIA_TYPE, &index.bytes, &index.digest)
                .map(|()| matches!(state, TagState::TakeOver(_)))
                .map_err(|error| error.to_string()),
            TagState::Unknown(reason) | TagState::Conflicting(reason) => Err(reason),
            TagState::Matching | TagState::Kept(_) => Ok(false),
        }
    }

    /// Push or move one tag that the inspection found, and record what happened in `step`.
    fn complete_tag(
        &self,
        tag: &OciTag,
        state: &TagState,
        step: &mut AssetStep,
        release: &Release,
        index: &RemoteIndex,
    ) -> Result<(), PublicationError> {
        if !matches!(
            state,
            TagState::Missing | TagState::Behind | TagState::TakeOver(_) | TagState::Unknown(_)
        ) {
            return Ok(());
        }
        if let OciTag::Alias(_) = tag {
            // Aliases come last and are best effort: a failure is a warning that a rerun
            // repairs, and never stops the release.
            let taken_over = match self.move_alias(tag, release, index) {
                Ok(taken_over) => taken_over,
                Err(reason) => {
                    *step = AssetStep::Warning(reason);
                    return Ok(());
                }
            };
            if let Ok((state, _)) = self.tag_state(tag, release) {
                *step = match state {
                    TagState::Matching if taken_over => AssetStep::TakenOver,
                    TagState::Matching => AssetStep::Uploaded,
                    TagState::Kept(reason) => AssetStep::Kept(reason),
                    TagState::Unknown(reason) | TagState::Conflicting(reason) => {
                        AssetStep::Warning(reason)
                    }
                    TagState::Missing | TagState::Behind | TagState::TakeOver(_) => {
                        AssetStep::Warning("the registry did not keep the alias".into())
                    }
                };
            }
            return Ok(());
        }
        // Look again right before writing: a concurrent run may have pushed the tag, and a
        // tag that now points elsewhere must not be overwritten. A tag that was missing is
        // reported as pushed even if it reappeared meanwhile, such as when a registry restores
        // a deleted version's tags once its index is pushed again.
        match self.tag_state(tag, release)?.0 {
            TagState::Missing | TagState::Behind | TagState::TakeOver(_) => {
                self.registry.put_manifest(
                    tag.name(),
                    INDEX_MEDIA_TYPE,
                    &index.bytes,
                    &index.digest,
                )?;
                *step = AssetStep::Uploaded;
            }
            TagState::Matching => *step = AssetStep::Uploaded,
            TagState::Kept(reason) => *step = AssetStep::Kept(reason),
            TagState::Conflicting(message) | TagState::Unknown(message) => {
                return Err(PublicationError::new(message));
            }
        }
        Ok(())
    }

    fn steps(&self, inspection: &Inspection, images: Vec<AssetStep>) -> ArtifactSteps {
        let mut steps: ArtifactSteps = self
            .platforms
            .keys()
            .zip(images)
            .map(|(platform, step)| ArtifactStep {
                kind: IMAGE,
                name: platform.to_string(),
                step,
            })
            .collect();
        steps.extend(inspection.tags.iter().map(|(tag, state)| ArtifactStep {
            kind: match tag {
                OciTag::Version(_) => TAG,
                OciTag::Alias(_) => ALIAS,
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
        progress: &mut dyn FnMut(&Platform, &AssetStep),
    ) -> Result<(RemoteIndex, Vec<AssetStep>), PublicationError> {
        if let Some(index) = &inspection.existing {
            for platform in self.platforms.keys() {
                progress(platform, &AssetStep::AlreadyUploaded);
            }
            return Ok((
                index.clone(),
                vec![AssetStep::AlreadyUploaded; self.platforms.len()],
            ));
        }
        let (images, steps): (Vec<_>, Vec<_>) = self
            .platforms
            .iter()
            .map(|(platform, path)| {
                let (image, step) = self.push_image(platform, path)?;
                progress(platform, &step);
                Ok((image, step))
            })
            .collect::<Result<Vec<_>, PublicationError>>()?
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

/// The credential variables of a publisher that are unset or empty, as `lookup` reads them; none
/// for a public repository, which `plan` and `prepare` read anonymously.
pub fn missing_credentials(
    declaration: &OciImageDeclaration,
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    declaration.registry().missing_credentials(lookup)
}

/// The trusted predecessor's version tag that points to `digest`, so an alias without a version annotation
/// that points there was pushed for that release by the tool before Presswerk. Both digests come
/// from the registry.
fn predecessor_tag(
    registry: &RegistryClient,
    tags: &[String],
    predecessor: Option<&Version>,
    digest: &str,
) -> Result<Option<String>, PublicationError> {
    let Some(predecessor) = predecessor else {
        return Ok(None);
    };
    for tag in render_tags(tags, &[], predecessor).map_err(PublicationError::new)? {
        if registry.manifest_digest(tag.name())?.as_deref() == Some(digest) {
            return Ok(Some(tag.name().to_owned()));
        }
    }
    Ok(None)
}

/// An alias that points to a manifest without a version annotation, so another tool pushed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignAlias {
    /// The alias with its repository, such as `ghcr.io/owner/project:latest`.
    pub reference: String,
    pub digest: String,
    /// The trusted predecessor's version tag that points to the same digest; the release then
    /// takes the alias over instead of keeping it.
    pub predecessor_tag: Option<String>,
}

/// The aliases of `version` that its release would keep because another tool pushed them, or
/// take over from the trusted `predecessor`. Read before anything is written, so `plan` and
/// `prepare` can warn about them; a pre-release moves no alias and has none.
pub fn foreign_aliases(
    registry: &RegistryClient,
    repository: &str,
    tags: &[String],
    aliases: &[String],
    version: &Version,
    predecessor: Option<&Version>,
) -> Result<Vec<ForeignAlias>, PublicationError> {
    if !version.pre.is_empty() {
        return Ok(Vec::new());
    }
    let mut foreign = Vec::new();
    for tag in render_tags(tags, aliases, version).map_err(PublicationError::new)? {
        let OciTag::Alias(alias) = tag else {
            continue;
        };
        let Some((digest, manifest)) = registry.manifest(&alias)? else {
            continue;
        };
        if annotated_version(&manifest).is_some() {
            continue;
        }
        let predecessor_tag = predecessor_tag(registry, tags, predecessor, &digest)?;
        foreign.push(ForeignAlias {
            reference: format!("{repository}:{alias}"),
            digest,
            predecessor_tag,
        });
    }
    Ok(foreign)
}

/// An OCI image publisher with its own registry client, such as one for a test registry.
impl Publisher for OciImagePublisher {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(
            &self.repository,
            self.platforms.keys().map(ToString::to_string),
            &self.tags,
            &self.aliases,
            version,
        )
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        completeness_with(
            &self.repository,
            &self.tags,
            &self.aliases,
            release,
            |tag| Ok(self.registry.manifest(tag)?.map(|(_, manifest)| manifest)),
        )
    }

    fn state(&self, release: &Release, _ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        let inspection = self.inspect(release)?;
        if let Some(conflict) = inspection.conflict() {
            return Err(PublicationError::new(conflict));
        }
        let (index, images) = self.release_index(&inspection, release, &mut |platform, step| {
            ctx.progress(&ArtifactStep {
                kind: IMAGE,
                name: platform.to_string(),
                step: step.clone(),
            });
        })?;

        let mut steps = self.steps(&inspection, images);
        let tag_steps = steps.len() - inspection.tags.len();
        for ((tag, state), step) in inspection.tags.iter().zip(&mut steps[tag_steps..]) {
            self.complete_tag(tag, state, &mut step.step, release, &index)?;
            ctx.progress(step);
        }

        match self.checked_state(release)? {
            PublisherState::Published | PublisherState::AliasesPending => Ok(steps),
            state => Err(PublicationError::new(format!(
                "{} was not published: {state:?}",
                self.repository
            ))),
        }
    }

    fn complete_steps(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        let inspection = self.inspect(release)?;
        let images = vec![AssetStep::AlreadyUploaded; self.platforms.len()];
        Ok(self.steps(&inspection, images))
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

impl OciImagePublisher {
    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_executable_inherits_no_secret() {
        let _secrets = crate::publishers::shared::secrets::Secrets::resolve_with(
            vec![(
                "oci-image.default.password".into(),
                crate::publishers::shared::secrets::SecretSource::Env(
                    "PRESSWERK_TEST_IMAGE_EXECUTABLE_SECRET".into(),
                ),
            )],
            // A multiline value, such as JSON registry credentials.
            |_| Some("{\n  \"password\": \"presswerk-multiline-sentinel\"\n}".into()),
        );
        let command = image_command(Path::new("/nix/store/abc-image"));
        assert!(command.get_envs().any(|(name, value)| {
            name == "PRESSWERK_TEST_IMAGE_EXECUTABLE_SECRET" && value.is_none()
        }));
    }

    #[test]
    fn image_executable_diagnostics_never_show_a_secret() {
        // Resolving a secret is what makes its value one that output never shows.
        let _secrets = crate::publishers::shared::secrets::Secrets::resolve_with(
            vec![(
                "oci-image.default.password".into(),
                crate::publishers::shared::secrets::SecretSource::Env(
                    "PRESSWERK_TEST_DIAGNOSTICS".into(),
                ),
            )],
            |_| Some("presswerk-diagnostics-sentinel".into()),
        );
        let mut sink = Vec::new();
        forward_diagnostics(
            &b"pushing with presswerk-diagnostics-sentinel\nsecond line"[..],
            &mut sink,
        );
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "pushing with [redacted]\nsecond line\n"
        );
    }

    #[test]
    fn an_unreadable_tag_is_unknown_not_incomplete() {
        let release = Release {
            commit: "0".repeat(40),
            version: Version::new(1, 2, 3),
            tag: "v1.2.3".into(),
            notes: String::new(),
        };
        let completeness = completeness_with(
            "ghcr.io/owner/project",
            &["{version}".into()],
            &[],
            &release,
            |_| Err(PublicationError::new("the registry answered HTTP 403")),
        )
        .unwrap();
        assert_eq!(
            completeness,
            Completeness::Unknown("the registry answered HTTP 403".into())
        );
        // A tag that is read and missing is incomplete.
        assert!(matches!(
            completeness_with(
                "ghcr.io/owner/project",
                &["{version}".into()],
                &[],
                &release,
                |_| Ok(None)
            )
            .unwrap(),
            Completeness::Incomplete { .. }
        ));
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
        // Renders to the version tag of every final release.
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
}
