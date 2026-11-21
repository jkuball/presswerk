//! The npm publisher: publishes a package tarball that the flake builds to an npm registry, with
//! one dist-tag, and moves alias dist-tags such as `latest` through the dist-tag API.
//!
//! Presswerk never packs: the tarball is published as it is. It reads only what the publish
//! request needs from it, its `package.json`, and compares the name and version found there with
//! the declaration and the release. A version that already exists matches when the registry's
//! `dist.integrity` (sha512) equals the tarball's.

use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::shared::alias::{self, Position};
use crate::publishers::shared::secrets::{Credential, Secrets};
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, shared,
};
use flate2::read::GzDecoder;
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;
use url::Url;

/// The package version of a release.
pub const PACKAGE: ArtifactKind = ArtifactKind::new("package", "published");
/// A dist-tag that follows the highest final version, such as `latest`.
pub const ALIAS: ArtifactKind = ArtifactKind::new("alias", "set");

const LABELS: Labels = Labels {
    publisher: "registry",
    artifacts: "dist-tags",
};

/// The npm documentation that states which dist-tags npm refuses.
pub const DIST_TAG_RULE: &str = "https://docs.npmjs.com/cli/commands/npm-dist-tag#caveats";

/// How long `publish` waits, one delay after another, for a registry to show a version it
/// accepted, since some serve package documents from a cache.
const VERIFY_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// `publishers.npm.<name>`: a package tarball published to an npm registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NpmDeclaration {
    /// The package name, such as `@scope/package`, which the tarball's `package.json` names too.
    pub name: String,
    /// The store path of the tarball, or of a directory that holds exactly one `.tgz`.
    pub package: String,
    /// The registry's base URL, such as `https://registry.npmjs.org`.
    pub registry: String,
    pub access: Access,
    /// The dist-tag sent with the upload; never moved afterwards.
    pub tag: String,
    /// Dist-tag templates that move to a higher final version, such as `latest`.
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<Credential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<Credential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Credential>,
}

/// Who may install the package, as the registry records it with the first upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Public,
    Restricted,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Restricted => "restricted",
        }
    }
}

/// How a registry is authenticated.
#[derive(Clone, PartialEq, Eq)]
pub enum Authentication {
    Token(String),
    Basic { username: String, password: String },
}

impl fmt::Debug for Authentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Token(_) => "Token([redacted])",
            Self::Basic { .. } => "Basic([redacted])",
        })
    }
}

impl Authentication {
    fn header(&self) -> String {
        match self {
            Self::Token(token) => format!("Bearer {token}"),
            Self::Basic { username, password } => {
                format!("Basic {}", shared::base64(format!("{username}:{password}")))
            }
        }
    }
}

/// Whether a dist-tag would be read as a version or a range, which npm refuses, such as `1.2`,
/// `v1`, or `x`. Approximates how npm's `semver.validRange` reads it; Nix checks the same.
#[must_use]
pub fn looks_like_version(tag: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s*(?:[<>]=?|=|~>?|\^)?\s*[vV=]?\s*(?:[0-9]|[xX*](?:$|[\s.|-]))")
            .expect("the version pattern is valid")
    });
    tag.trim().is_empty() || PATTERN.is_match(tag)
}

fn version_like_error(what: &str, tag: &str) -> String {
    format!(
        "npm {what} {tag:?} would be read as a version or range, which npm refuses as a dist-tag ({DIST_TAG_RULE}); use a name that starts with a letter other than v"
    )
}

impl NpmDeclaration {
    fn validate(&self) -> Result<(), String> {
        let valid_name = match self.name.split_once('/') {
            None => !self.name.is_empty() && !self.name.starts_with('@'),
            Some((scope, name)) => {
                scope.len() > 1 && scope.starts_with('@') && !name.is_empty() && !name.contains('/')
            }
        };
        if !valid_name {
            return Err(format!(
                "npm package name {:?} must be a name or @scope/name",
                self.name
            ));
        }
        if !self.package.starts_with('/') {
            return Err(format!(
                "the npm package must be an absolute store path, not {:?}",
                self.package
            ));
        }
        registry_url(&self.registry)?;
        if looks_like_version(&self.tag) {
            return Err(version_like_error("tag", &self.tag));
        }
        if let Some(alias) = alias::single_version_template(&self.aliases) {
            return Err(format!(
                "npm alias {alias:?} must not contain {{version}}, which names a single version; the release's own dist-tag is tag"
            ));
        }
        render_aliases(&self.tag, &self.aliases, &Version::new(1, 2, 3))?;
        self.authentication_shape()
    }

    fn authentication_shape(&self) -> Result<(), String> {
        let invalid = |error: DeclarationError| error.to_string();
        match (&self.token, &self.username, &self.password) {
            (Some(token), None, None) => token.validate_secret("npm token").map_err(invalid),
            (None, Some(username), Some(password)) => {
                username.validate().map_err(invalid)?;
                password
                    .validate_secret("npm registry password")
                    .map_err(invalid)
            }
            _ => Err("an npm publisher needs either token, or username and password".into()),
        }
    }

    /// The registry's authentication, from the resolved secrets.
    fn authentication(&self, secrets: &Secrets) -> Result<Authentication, PublicationError> {
        match (&self.token, &self.username, &self.password) {
            (Some(token), _, _) => Ok(Authentication::Token(secrets.get(token)?)),
            (None, Some(username), Some(password)) => Ok(Authentication::Basic {
                username: secrets.get(username)?,
                password: secrets.get(password)?,
            }),
            _ => Err(PublicationError::new(
                "an npm publisher needs either token, or username and password",
            )),
        }
    }

    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        [
            ("token", &self.token),
            ("username", &self.username),
            ("password", &self.password),
        ]
        .into_iter()
        .filter_map(|(option, credential)| {
            credential.as_ref().map(|credential| (option, credential))
        })
        .collect()
    }
}

/// The registry's base URL: HTTP(S), without credentials, query, or fragment.
fn registry_url(registry: &str) -> Result<Url, String> {
    let url = Url::parse(registry)
        .map_err(|error| format!("invalid npm registry URL {registry:?}: {error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "npm registry {registry:?} must be an HTTP(S) base URL without credentials, query, or fragment"
        ));
    }
    Ok(url)
}

/// Renders the alias templates for a version and checks that each is a dist-tag npm accepts,
/// other than `tag`, and declared once.
pub fn render_aliases(
    tag: &str,
    aliases: &[String],
    version: &Version,
) -> Result<Vec<String>, String> {
    let mut seen = HashSet::from([tag.to_owned()]);
    let rendered = alias::render(aliases, version);
    for (template, alias) in aliases.iter().zip(&rendered) {
        if looks_like_version(alias) {
            return Err(version_like_error(
                "alias",
                &format!("{template}, which renders to {alias} for version {version},"),
            ));
        }
        if alias == tag {
            return Err(format!(
                "npm alias {template:?} is the tag {tag:?}; the tag is set with the upload and never moved, so it cannot also be an alias"
            ));
        }
        if !seen.insert(alias.clone()) {
            return Err(format!(
                "npm alias {alias:?} is declared twice for version {version}"
            ));
        }
    }
    Ok(rendered)
}

/// What a registry knows about a package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packument {
    /// Every dist-tag with the version it points to.
    pub dist_tags: BTreeMap<String, String>,
    /// Every version with its `dist.integrity`, if the registry reports one.
    pub versions: BTreeMap<String, Option<String>>,
}

impl Packument {
    /// Decodes a package document, full or abbreviated.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Dist {
            integrity: Option<String>,
        }
        #[derive(Deserialize)]
        struct Manifest {
            dist: Option<Dist>,
        }
        #[derive(Deserialize)]
        struct Document {
            #[serde(rename = "dist-tags", default)]
            dist_tags: BTreeMap<String, String>,
            #[serde(default)]
            versions: BTreeMap<String, Manifest>,
        }
        let document: Document =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        Ok(Self {
            dist_tags: document.dist_tags,
            versions: document
                .versions
                .into_iter()
                .map(|(version, manifest)| (version, manifest.dist.and_then(|dist| dist.integrity)))
                .collect(),
        })
    }
}

/// Why a dist-tag could not be set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistTagError {
    /// The registry has no dist-tag API, with what it answered.
    Unsupported(String),
    Failed(String),
}

/// The requests the npm publisher makes, so tests can replace the registry.
pub trait NpmRegistry: fmt::Debug + Send + Sync {
    /// The package document of `name`; `None` if the package does not exist.
    fn packument(&self, name: &str) -> Result<Option<Packument>, PublicationError>;
    /// Uploads a new version, as `npm publish` does.
    fn publish(&self, name: &str, document: &Value) -> Result<(), PublicationError>;
    /// Points the dist-tag `tag` of `name` at `version`.
    fn set_dist_tag(&self, name: &str, tag: &str, version: &str) -> Result<(), DistTagError>;
}

/// An npm registry over HTTP.
#[derive(Clone)]
pub struct RegistryClient {
    agent: ureq::Agent,
    base: Url,
    authentication: Option<Authentication>,
}

impl fmt::Debug for RegistryClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryClient")
            .field("base", &self.base.as_str())
            .field("authentication", &self.authentication)
            .finish_non_exhaustive()
    }
}

impl RegistryClient {
    /// A client for the registry at `registry`, anonymous without `authentication`.
    pub fn new(
        registry: &str,
        authentication: Option<Authentication>,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            base: registry_url(registry).map_err(PublicationError::new)?,
            authentication,
        })
    }

    /// The URL of `segments` below the registry; a scoped name is one segment, `@scope%2Fname`.
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("the registry URL is a base")
            .pop_if_empty()
            .extend(segments);
        url
    }

    fn request(&self, method: &str, url: &Url) -> ureq::Request {
        let request = self
            .agent
            .request(method, url.as_str())
            .set("User-Agent", "presswerk");
        match &self.authentication {
            Some(authentication) => request.set("Authorization", &authentication.header()),
            None => request,
        }
    }
}

/// What a failed request answered, for an error message.
fn failure(error: ureq::Error) -> (Option<u16>, String) {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|body| {
                    body.get("error")
                        .or_else(|| body.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            let detail = detail.trim();
            let message = if detail.is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}: {detail}")
            };
            (Some(status), shared::secrets::redact(&message))
        }
        ureq::Error::Transport(error) => (None, shared::secrets::redact(&error.to_string())),
    }
}

impl NpmRegistry for RegistryClient {
    fn packument(&self, name: &str) -> Result<Option<Packument>, PublicationError> {
        let url = self.url(&[name]);
        let response = self
            .request("GET", &url)
            // The abbreviated document holds every integrity and dist-tag; registries without it
            // answer with the full one.
            .set(
                "Accept",
                "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8, */*",
            )
            .set("Cache-Control", "no-cache")
            .call();
        match response {
            Ok(response) => {
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        PublicationError::new(format!("could not read {name} from {url}: {error}"))
                    })?;
                Packument::from_json(&bytes).map(Some).map_err(|error| {
                    PublicationError::new(format!("could not decode {name} from {url}: {error}"))
                })
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(error) => Err(PublicationError::new(format!(
                "could not read {name} from {url}: {}",
                failure(error).1
            ))),
        }
    }

    fn publish(&self, name: &str, document: &Value) -> Result<(), PublicationError> {
        let url = self.url(&[name]);
        self.request("PUT", &url)
            .set("npm-command", "publish")
            .send_json(document)
            .map(drop)
            .map_err(|error| {
                PublicationError::new(format!(
                    "{url} refused to publish {name}: {}",
                    failure(error).1
                ))
            })
    }

    fn set_dist_tag(&self, name: &str, tag: &str, version: &str) -> Result<(), DistTagError> {
        let url = self.url(&["-", "package", name, "dist-tags", tag]);
        match self
            .request("PUT", &url)
            .set("npm-command", "dist-tag")
            .send_json(json!(version))
        {
            Ok(_) => Ok(()),
            Err(error) => match failure(error) {
                (Some(404 | 405 | 501), message) => Err(DistTagError::Unsupported(message)),
                (_, message) => Err(DistTagError::Failed(format!(
                    "could not set dist-tag {tag} of {name}: {message}"
                ))),
            },
        }
    }
}

/// The tarball of a release, with what its `package.json` says and its digests.
#[derive(Debug, Clone)]
pub struct LocalPackage {
    bytes: Vec<u8>,
    manifest: Map<String, Value>,
    /// The `sha512-…` integrity, as registries report it.
    integrity: String,
    /// The SHA-1 digest in hex, which registries still expect as `dist.shasum`.
    shasum: String,
}

impl LocalPackage {
    /// Reads the tarball at `path`, or the one `.tgz` in the directory at `path`.
    pub fn read(path: &Path) -> Result<Self, PublicationError> {
        let tarball = tarball_path(path).map_err(PublicationError::new)?;
        let bytes = std::fs::read(&tarball).map_err(|error| {
            PublicationError::new(format!(
                "could not read npm package {}: {error}",
                tarball.display()
            ))
        })?;
        Self::from_bytes(bytes).map_err(|error| {
            PublicationError::new(format!("npm package {}: {error}", tarball.display()))
        })
    }

    /// Reads a gzipped package tarball, whose `package.json` lies in its top directory, such as
    /// `package/package.json`.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
        let mut manifest = None;
        for entry in archive.entries().map_err(|error| error.to_string())? {
            let mut entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path().map_err(|error| error.to_string())?;
            let components: Vec<_> = path.components().collect();
            if components.len() != 2 || components[1].as_os_str() != "package.json" {
                continue;
            }
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            manifest = Some(
                serde_json::from_slice::<Map<String, Value>>(&contents)
                    .map_err(|error| format!("package.json is no JSON object: {error}"))?,
            );
            break;
        }
        let manifest = manifest.ok_or("the tarball holds no package.json in its top directory")?;
        for field in ["name", "version"] {
            if !manifest.get(field).is_some_and(Value::is_string) {
                return Err(format!("package.json has no {field}"));
            }
        }
        Ok(Self {
            integrity: format!("sha512-{}", shared::base64(Sha512::digest(&bytes))),
            shasum: Sha1::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            bytes,
            manifest,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.manifest["name"].as_str().unwrap_or_default()
    }

    #[must_use]
    pub fn version(&self) -> &str {
        self.manifest["version"].as_str().unwrap_or_default()
    }

    #[must_use]
    pub fn integrity(&self) -> &str {
        &self.integrity
    }

    /// The document `npm publish` sends for this tarball: the version's manifest, the dist-tag
    /// `tag`, and the tarball as an attachment.
    #[must_use]
    pub fn publish_document(&self, registry: &str, tag: &str, access: Access) -> Value {
        let name = self.name();
        let version = self.version();
        let filename = format!("{name}-{version}.tgz");
        let mut manifest = self.manifest.clone();
        manifest.insert("_id".into(), json!(format!("{name}@{version}")));
        let mut dist = manifest
            .remove("dist")
            .and_then(|dist| match dist {
                Value::Object(dist) => Some(dist),
                _ => None,
            })
            .unwrap_or_default();
        dist.insert("integrity".into(), json!(self.integrity));
        dist.insert("shasum".into(), json!(self.shasum));
        dist.insert(
            "tarball".into(),
            json!(format!(
                "{}/{name}/-/{filename}",
                registry.trim_end_matches('/')
            )),
        );
        manifest.insert("dist".into(), Value::Object(dist));
        let mut document = json!({
            "_id": name,
            "name": name,
            "dist-tags": { tag: version },
            "versions": { version: manifest },
            "access": access.as_str(),
            "_attachments": {
                filename: {
                    "content_type": "application/octet-stream",
                    "data": shared::base64(&self.bytes),
                    "length": self.bytes.len(),
                }
            }
        });
        if let Some(description) = self.manifest.get("description") {
            document["description"] = description.clone();
        }
        document
    }
}

/// The tarball at `path`: the path itself, or the one `.tgz` in the directory at `path`.
fn tarball_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Ok(path.to_owned());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("could not list {}: {error}", path.display()))?;
    let mut tarballs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not list {}: {error}", path.display()))?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "tgz")
        {
            tarballs.push(entry.path());
        }
    }
    match tarballs.as_slice() {
        [tarball] => Ok(tarball.clone()),
        _ => Err(format!(
            "{} must be a package tarball or a directory with exactly one .tgz, but holds {}",
            path.display(),
            tarballs.len()
        )),
    }
}

/// Whether a registry's `dist.integrity` names the same tarball as `local`, a `sha512-…`
/// integrity. Without a sha512 hash, it cannot tell.
fn integrity_matches(remote: &str, local: &str) -> Option<bool> {
    let hashes: Vec<_> = remote
        .split_whitespace()
        .filter(|hash| hash.starts_with("sha512-"))
        // Options after `?` are no part of the hash.
        .map(|hash| hash.split('?').next().unwrap_or(hash))
        .collect();
    (!hashes.is_empty()).then(|| hashes.contains(&local))
}

/// The `npm` publisher type.
#[derive(Debug)]
pub struct Npm {
    declaration: NpmDeclaration,
    /// Built on first use, once the credentials are known.
    runner: OnceLock<NpmPublisher>,
}

impl Npm {
    #[must_use]
    pub fn declaration(&self) -> &NpmDeclaration {
        &self.declaration
    }

    fn runner(&self, ctx: &Context) -> Result<&NpmPublisher, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let declaration = &self.declaration;
        let client = RegistryClient::new(
            &declaration.registry,
            Some(declaration.authentication(&ctx.session.secrets)?),
        )?;
        let runner = NpmPublisher::new(Box::new(client), declaration.clone());
        Ok(self.runner.get_or_init(|| runner))
    }
}

impl PublisherType for Npm {
    const NAME: &'static str = "npm";

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: NpmDeclaration = shared::decode(declaration)?;
        declaration.validate().map_err(DeclarationError::Invalid)?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

impl Publisher for Npm {
    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        self.declaration.credentials()
    }

    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.declaration, version)
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        vec![("package".into(), self.declaration.package.clone())]
    }

    /// Reads with the credentials if they are set, and anonymously otherwise, as for a public
    /// package.
    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        let declaration = &self.declaration;
        let secrets = &ctx.session.secrets;
        let authentication = if declaration
            .credentials()
            .iter()
            .all(|(_, credential)| secrets.is_set(credential))
        {
            Some(declaration.authentication(secrets)?)
        } else {
            None
        };
        let client = RegistryClient::new(&declaration.registry, authentication)?;
        Ok(completeness_with(&client, declaration, release))
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

/// What a release of `version` writes: the version, the dist-tag it is uploaded with, and the
/// aliases.
fn describe(declaration: &NpmDeclaration, version: &Version) -> Result<Description, String> {
    let aliases = render_aliases(&declaration.tag, &declaration.aliases, version)?;
    Ok(Description {
        target: format!(
            "{}/{}",
            declaration.registry.trim_end_matches('/'),
            declaration.name
        ),
        items: [
            (
                "version".to_owned(),
                format!("{}@{version}", declaration.name),
            ),
            ("tag".to_owned(), declaration.tag.clone()),
        ]
        .into_iter()
        .chain(aliases.into_iter().map(|alias| ("alias".to_owned(), alias)))
        .collect(),
    })
}

/// Whether the release's version exists in the registry. Its integrity is not compared, since
/// the previous release's tarball is not fetched, and neither the tag nor the aliases count. A
/// registry that cannot be read makes the release `Unknown`.
pub fn completeness_with(
    registry: &dyn NpmRegistry,
    declaration: &NpmDeclaration,
    release: &Release,
) -> Completeness {
    let name = &declaration.name;
    let version = release.version.to_string();
    let reason = match registry.packument(name) {
        Err(error) => return Completeness::Unknown(error.to_string()),
        Ok(Some(packument)) if packument.versions.contains_key(&version) => {
            return Completeness::Complete;
        }
        Ok(Some(_)) => "the version is missing",
        Ok(None) => "the package does not exist",
    };
    Completeness::Incomplete {
        target: format!("npm package {name}@{version} in {}", declaration.registry),
        reason: reason.into(),
    }
}

/// What the registry holds for the release's version.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionState {
    Missing,
    Matching,
    Conflicting(String),
}

/// The registry's state for a release, read before anything is written.
#[derive(Debug)]
struct Inspection {
    version: VersionState,
    aliases: Vec<(String, Position)>,
}

/// Publishes one npm package version and moves its aliases.
#[derive(Debug)]
pub struct NpmPublisher {
    registry: Box<dyn NpmRegistry>,
    declaration: NpmDeclaration,
    verify_delays: Vec<Duration>,
    /// Read on first use.
    local: OnceLock<LocalPackage>,
}

impl NpmPublisher {
    #[must_use]
    pub fn new(registry: Box<dyn NpmRegistry>, declaration: NpmDeclaration) -> Self {
        Self {
            registry,
            declaration,
            verify_delays: VERIFY_DELAYS.to_vec(),
            local: OnceLock::new(),
        }
    }

    /// How long to wait, one delay after another, for the registry to show a published version.
    #[must_use]
    pub fn with_verify_delays(mut self, delays: Vec<Duration>) -> Self {
        self.verify_delays = delays;
        self
    }

    /// The tarball, checked against the declaration's name and the release's version, which the
    /// publish needs anyway.
    fn local(&self, release: &Release) -> Result<&LocalPackage, PublicationError> {
        let local = match self.local.get() {
            Some(local) => local,
            None => {
                let local = LocalPackage::read(Path::new(&self.declaration.package))?;
                self.local.get_or_init(|| local)
            }
        };
        if local.name() != self.declaration.name {
            return Err(PublicationError::new(format!(
                "the npm package {} is named {:?} in its package.json, not {:?}",
                self.declaration.package,
                local.name(),
                self.declaration.name
            )));
        }
        if Version::parse(local.version()).ok().as_ref() != Some(&release.version) {
            return Err(PublicationError::new(format!(
                "the npm package {} has version {:?} in its package.json, not the release version {}",
                self.declaration.package,
                local.version(),
                release.version
            )));
        }
        Ok(local)
    }

    fn reference(&self, version: &Version) -> String {
        format!("{}@{version}", self.declaration.name)
    }

    fn inspect(
        &self,
        release: &Release,
        local: &LocalPackage,
    ) -> Result<Inspection, PublicationError> {
        let packument = self
            .registry
            .packument(&self.declaration.name)?
            .unwrap_or_default();
        let version = match packument.versions.get(&release.version.to_string()) {
            None => VersionState::Missing,
            Some(remote) => match remote
                .as_deref()
                .and_then(|remote| integrity_matches(remote, local.integrity()))
            {
                Some(true) => VersionState::Matching,
                Some(false) => VersionState::Conflicting(format!(
                    "{} exists in {} with another tarball: its integrity is {}, the release's {}",
                    self.reference(&release.version),
                    self.declaration.registry,
                    remote.as_deref().unwrap_or_default(),
                    local.integrity()
                )),
                None => {
                    return Err(PublicationError::new(format!(
                        "{} exists in {}, but the registry reports no sha512 integrity to compare it with",
                        self.reference(&release.version),
                        self.declaration.registry
                    )));
                }
            },
        };
        let aliases = self.alias_positions(&packument, &release.version)?;
        Ok(Inspection { version, aliases })
    }

    fn alias_positions(
        &self,
        packument: &Packument,
        version: &Version,
    ) -> Result<Vec<(String, Position)>, PublicationError> {
        let aliases = render_aliases(&self.declaration.tag, &self.declaration.aliases, version)
            .map_err(PublicationError::new)?;
        Ok(aliases
            .into_iter()
            .map(|alias| {
                let position = match packument.dist_tags.get(&alias) {
                    None => alias::position(None, version),
                    Some(current) => match Version::parse(current) {
                        Ok(current) => alias::position(Some(&current), version),
                        Err(_) => Position::Kept(format!("points to {current:?}, no version")),
                    },
                };
                (alias, position)
            })
            .collect())
    }

    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        let local = self.local(release)?;
        let inspection = self.inspect(release, local)?;
        Ok(match inspection.version {
            VersionState::Conflicting(reason) => PublisherState::Conflicting(reason),
            VersionState::Missing => PublisherState::Missing,
            VersionState::Matching
                if inspection
                    .aliases
                    .iter()
                    .all(|(_, position)| !position.moves()) =>
            {
                PublisherState::Published
            }
            VersionState::Matching => PublisherState::AliasesPending,
        })
    }

    fn alias_step(&self, alias: &str, position: &Position) -> ArtifactStep {
        ArtifactStep {
            kind: ALIAS,
            name: format!("{}@{alias}", self.declaration.name),
            step: match position {
                Position::Kept(reason) => AssetStep::Kept(reason.clone()),
                _ => AssetStep::AlreadyUploaded,
            },
        }
    }

    /// Uploads the version if it is missing. A failed upload is inspected again, since a
    /// concurrent run may have published the same tarball.
    fn upload(
        &self,
        release: &Release,
        local: &LocalPackage,
        inspection: &Inspection,
    ) -> Result<AssetStep, PublicationError> {
        match &inspection.version {
            VersionState::Matching => return Ok(AssetStep::AlreadyUploaded),
            VersionState::Conflicting(reason) => return Err(PublicationError::new(reason)),
            VersionState::Missing => {}
        }
        let declaration = &self.declaration;
        let document =
            local.publish_document(&declaration.registry, &declaration.tag, declaration.access);
        if let Err(error) = self.registry.publish(&declaration.name, &document) {
            match self.inspect(release, local).map(|again| again.version) {
                Ok(VersionState::Matching) => {}
                Ok(VersionState::Conflicting(reason)) => return Err(PublicationError::new(reason)),
                _ => return Err(error),
            }
        }
        Ok(AssetStep::Uploaded)
    }

    /// Moves every alias that points to a lower version or to nothing, after reading them again.
    /// Aliases are best effort: a failure is a warning that a rerun repairs.
    fn move_aliases(&self, release: &Release) -> Vec<ArtifactStep> {
        let version = &release.version;
        let positions = self
            .registry
            .packument(&self.declaration.name)
            .and_then(|packument| self.alias_positions(&packument.unwrap_or_default(), version));
        let positions = match positions {
            Ok(positions) => positions,
            Err(error) => {
                return render_aliases(&self.declaration.tag, &self.declaration.aliases, version)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|alias| ArtifactStep {
                        step: AssetStep::Warning(error.to_string()),
                        ..self.alias_step(&alias, &Position::Same)
                    })
                    .collect();
            }
        };
        positions
            .into_iter()
            .map(|(alias, position)| {
                let mut step = self.alias_step(&alias, &position);
                if position.moves() {
                    step.step = match self.registry.set_dist_tag(
                        &self.declaration.name,
                        &alias,
                        &version.to_string(),
                    ) {
                        Ok(()) => AssetStep::Uploaded,
                        Err(DistTagError::Unsupported(answer)) => AssetStep::Warning(format!(
                            "{} has no dist-tag API ({answer}); set {alias} another way, or publish with tag = \"{alias}\" and no aliases",
                            self.declaration.registry
                        )),
                        Err(DistTagError::Failed(reason)) => AssetStep::Warning(reason),
                    };
                }
                step
            })
            .collect()
    }
}

impl Publisher for NpmPublisher {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.declaration, version)
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        Ok(completeness_with(
            self.registry.as_ref(),
            &self.declaration,
            release,
        ))
    }

    fn state(&self, release: &Release, _ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        let local = self.local(release)?;
        let inspection = self.inspect(release, local)?;
        let package = ArtifactStep {
            kind: PACKAGE,
            name: self.reference(&release.version),
            step: self.upload(release, local, &inspection)?,
        };
        ctx.progress(&package);
        let mut steps = vec![package];

        // Verify before the aliases move, so none points to a version the registry lost.
        let mut delays = self.verify_delays.iter();
        loop {
            let state = self.checked_state(release)?;
            match (&state, delays.next()) {
                (PublisherState::Published | PublisherState::AliasesPending, _) => break,
                (PublisherState::Missing, Some(delay)) => std::thread::sleep(*delay),
                _ => {
                    return Err(PublicationError::new(format!(
                        "{} was not published: {state:?}",
                        self.reference(&release.version)
                    )));
                }
            }
        }

        for step in self.move_aliases(release) {
            ctx.progress(&step);
            steps.push(step);
        }
        Ok(steps)
    }

    fn complete_steps(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        let local = self.local(release)?;
        let inspection = self.inspect(release, local)?;
        Ok(std::iter::once(ArtifactStep {
            kind: PACKAGE,
            name: self.reference(&release.version),
            step: AssetStep::AlreadyUploaded,
        })
        .chain(
            inspection
                .aliases
                .iter()
                .map(|(alias, position)| self.alias_step(alias, position)),
        )
        .collect())
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

#[cfg(test)]
mod tests;
