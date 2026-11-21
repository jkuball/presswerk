//! The publisher types: each writes a release to one kind of remote.
//!
//! The core knows no type. It decodes `publishers.<type>.<name>` generically, resolves `<type>`
//! through [`TYPES`], and talks to each instance only through [`Publisher`]. Every value that
//! crosses this boundary is serializable, so a type could also be implemented outside the CLI,
//! with JSON in and out.

pub mod cargo;
pub mod github_release;
pub mod oci;
pub mod shared;

use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactStep, ArtifactSteps, PublicationError, PublisherNode, PublisherState, Release,
};
use crate::publishers::shared::secrets::{Credential, SecretSource};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// Every publisher type this CLI knows, in the order `plan` lists and `publish` reports them.
pub const TYPES: &[Registration] = &[
    Registration::of::<oci::image::OciImage>(),
    Registration::of::<oci::helm::OciHelm>(),
    Registration::of::<github_release::GithubRelease>(),
    Registration::of::<cargo::Cargo>(),
];

/// One declared publisher of a release, such as one GitHub Release or one OCI repository.
pub trait Publisher: Send + Sync {
    /// What the release of `version` writes, for `presswerk plan`. Pure: it fails if the
    /// declaration cannot express that version, such as a tag template that renders an invalid
    /// tag, so `plan`, `prepare`, and `publish` stop before anything is written.
    fn describe(&self, version: &Version, tag: &str) -> Result<Description, String>;

    /// The local files the release needs, as store paths with a label, such as `asset tool`.
    /// `publish` fetches them before anything is written.
    fn artifacts(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The credentials of the declaration, by option name, such as `token`. The core resolves
    /// every secret among them before the publisher runs and redacts their values from output.
    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        Vec::new()
    }

    /// Whether `release` is complete on the remote, without local artifacts: `plan`, `prepare`,
    /// and `publish` check the previous release this way.
    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError>;

    /// Reads the remote for `release` before anything is written, to warn through the context
    /// about what `publish` will find. Best effort: it never fails.
    fn preview(&self, _release: &Release, _ctx: &Context) {}

    /// The remote state compared with the local artifacts.
    fn state(&self, release: &Release, ctx: &Context) -> Result<PublisherState, PublicationError>;

    /// Creates every missing part, reporting each artifact through the context as soon as it is
    /// complete.
    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError>;

    /// The artifacts of a release whose state is already published, without writing.
    fn complete_steps(
        &self,
        _release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        Ok(Vec::new())
    }

    /// How `publish` names this publisher and its artifacts in its output.
    fn labels(&self) -> Labels {
        Labels::default()
    }
}

/// A publisher type: the key under `publishers` and how one instance is read from it.
pub trait PublisherType: Publisher + Sized + 'static {
    /// The key under `publishers.<NAME>`, such as `github-release` or `oci-image`.
    const NAME: &'static str;
    /// Earlier names of the key that declarations may still use.
    const ALIASES: &'static [&'static str] = &[];

    /// Parses and validates one instance, without the core's `after` and `claims`. Pure: no
    /// environment, network, or store access.
    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError>;
}

/// A publisher type in [`TYPES`].
pub struct Registration {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    decode: fn(&Value) -> Result<Box<dyn Publisher>, DeclarationError>,
}

impl Registration {
    #[must_use]
    pub const fn of<T: PublisherType>() -> Self {
        Self {
            name: T::NAME,
            aliases: T::ALIASES,
            decode: decode::<T>,
        }
    }

    /// The registered type for a key under `publishers`, also by an earlier name.
    #[must_use]
    pub fn find(name: &str) -> Option<&'static Self> {
        TYPES
            .iter()
            .find(|registration| registration.name == name || registration.aliases.contains(&name))
    }
}

fn decode<T: PublisherType>(declaration: &Value) -> Result<Box<dyn Publisher>, DeclarationError> {
    Ok(Box::new(T::from_declaration(declaration)?))
}

/// What a release writes for one publisher, for `presswerk plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    /// Where it writes, such as `ghcr.io/owner/project`.
    pub target: String,
    /// Each thing it writes, by kind, such as `("tag", "1.2.3")` or `("asset", "tool")`.
    pub items: Vec<(String, String)>,
}

/// A remote object a publisher writes, such as one tag in one OCI repository. Each type's Nix
/// module derives the claims of an instance into the declaration, beside `after`, and checks them
/// with a sample version; the CLI checks them again with the version it releases, since some only
/// collide for certain versions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// The remote, such as `oci://ghcr.io/owner/project`.
    pub destination: String,
    /// The object within it, as a version template such as the tag `{major}.{minor}`.
    pub key: String,
}

/// Whether a previous release is complete on the remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Completeness {
    Complete,
    /// Something of the release is missing or different.
    Incomplete {
        /// What is incomplete, such as `ghcr.io/owner/project:1.2.3`.
        target: String,
        reason: String,
    },
    /// The state cannot be determined right now, such as when a registry is unreachable. It
    /// blocks like an incomplete release.
    Unknown(String),
}

/// How `publish` names a publisher and its artifacts in its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labels {
    /// The kind of remote, such as `release` or `registry`.
    pub publisher: &'static str,
    /// What it completes for an existing release, such as `assets` or `tags`.
    pub artifacts: &'static str,
}

impl Default for Labels {
    fn default() -> Self {
        Self {
            publisher: "publisher",
            artifacts: "artifacts",
        }
    }
}

/// Something a publisher found that the user should know, such as an alias another tool pushed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub level: WarningLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WarningLevel {
    Note,
    Warning,
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            WarningLevel::Note => "note",
            WarningLevel::Warning => "warning",
        };
        write!(formatter, "{level}: {}", self.message)
    }
}

/// What every publisher of one run shares: the resolved secrets and facts about the release.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub secrets: shared::secrets::Secrets,
    /// The changelog's repository path, for release bodies that link to it.
    pub changelog: String,
    /// The version of a trusted predecessor released by another tool, whose leftovers the
    /// release takes over.
    pub trusted_predecessor: Option<Version>,
}

/// What a publisher gets for one call: the session, its own address, and where to report.
pub struct Context<'a> {
    pub session: &'a Session,
    /// The publisher's address, such as `oci-image.default`.
    pub address: &'a str,
    progress: &'a (dyn Fn(&ArtifactStep) + Sync),
    warnings: &'a (dyn Fn(Warning) + Sync),
}

impl Session {
    /// The session for the publishers of `declaration`.
    #[must_use]
    pub fn for_declaration(declaration: &crate::declaration::ReleaseDeclaration) -> Self {
        Self {
            secrets: shared::secrets::Secrets::resolve(declaration.publishers.secrets()),
            changelog: declaration.changelog.path.clone(),
            trusted_predecessor: None,
        }
    }
}

static DETACHED: std::sync::LazyLock<Session> = std::sync::LazyLock::new(Session::default);

impl<'a> Context<'a> {
    #[must_use]
    pub fn new(session: &'a Session, address: &'a str) -> Self {
        Self {
            session,
            address,
            progress: &|_| {},
            warnings: &|_| {},
        }
    }

    /// A context without secrets that drops every report, for publishers that bring their own
    /// clients.
    #[must_use]
    pub fn detached() -> Context<'static> {
        Context::new(&DETACHED, "")
    }

    #[must_use]
    pub fn with_progress(mut self, progress: &'a (dyn Fn(&ArtifactStep) + Sync)) -> Self {
        self.progress = progress;
        self
    }

    #[must_use]
    pub fn with_warnings(mut self, warnings: &'a (dyn Fn(Warning) + Sync)) -> Self {
        self.warnings = warnings;
        self
    }

    /// Reports one artifact as soon as it is complete.
    pub fn progress(&self, step: &ArtifactStep) {
        (self.progress)(step);
    }

    pub fn warn(&self, level: WarningLevel, message: impl Into<String>) {
        (self.warnings)(Warning {
            level,
            message: message.into(),
        });
    }
}

/// The publishers of a declaration, as `publishers.<type>.<name>` JSON. Each type validates its
/// own instances; the core only knows `after`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Publishers(BTreeMap<String, BTreeMap<String, Value>>);

impl Publishers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares the instance `name` of `kind`, replacing an earlier one.
    pub fn insert(&mut self, kind: &str, name: &str, declaration: impl Serialize) {
        let declaration =
            serde_json::to_value(declaration).expect("publisher declarations serialize to JSON");
        self.0
            .entry(kind.to_owned())
            .or_default()
            .insert(name.to_owned(), declaration);
    }

    /// The declaration of the instance `name` of `kind`, as declared.
    #[must_use]
    pub fn get(&self, kind: &str, name: &str) -> Option<&Value> {
        self.0.get(kind)?.get(name)
    }

    /// Removes every instance of `kind`.
    pub fn remove_type(&mut self, kind: &str) {
        self.0.remove(kind);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.values().all(BTreeMap::is_empty)
    }

    /// The claims of every instance, by address, in [`TYPES`] order, then by name. Declarations
    /// written before claims existed have none.
    pub fn claims(&self) -> Result<Vec<(String, Claim)>, DeclarationError> {
        let mut claims = Vec::new();
        for registration in TYPES {
            for (name, declaration) in self.0.get(registration.name).into_iter().flatten() {
                let address = format!("{}.{name}", registration.name);
                let Some(declared) = declaration.get("claims") else {
                    continue;
                };
                let declared: Vec<Claim> = Vec::deserialize(declared)
                    .map_err(|error| shared::incompatible(format!("{address}.claims: {error}")))?;
                claims.extend(declared.into_iter().map(|claim| (address.clone(), claim)));
            }
        }
        Ok(claims)
    }

    /// The instances in [`TYPES`] order, then by name, each decoded through its type. An unknown
    /// type fails, so nothing is written for a declaration this CLI cannot fully read.
    /// Every secret the publishers declare, by where it is declared, such as
    /// `github-release.default.token`. Each type names its own credentials, so no other option
    /// is mistaken for one. Publishers this CLI cannot read declare none; validation reports them.
    #[must_use]
    pub fn secrets(&self) -> Vec<(String, SecretSource)> {
        let Ok(nodes) = self.nodes() else {
            return Vec::new();
        };
        nodes
            .iter()
            .flat_map(|node| {
                node.publisher
                    .credentials()
                    .into_iter()
                    .filter_map(|(option, credential)| match credential {
                        Credential::Secret(source) => {
                            Some((format!("{}.{option}", node.name), source.clone()))
                        }
                        Credential::Value(_) => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn nodes(&self) -> Result<Vec<PublisherNode>, DeclarationError> {
        for kind in self.0.keys() {
            if Registration::find(kind).is_none() {
                return Err(shared::incompatible(format!(
                    "unknown publisher type {kind:?}"
                )));
            }
        }
        let mut nodes = Vec::new();
        for registration in TYPES {
            let Some(instances) = self.0.get(registration.name) else {
                continue;
            };
            for (name, declaration) in instances {
                let address = format!("{}.{name}", registration.name);
                if name.is_empty() {
                    return Err(DeclarationError::Invalid(format!(
                        "{} publisher names must not be empty",
                        registration.name
                    )));
                }
                let mut declaration = declaration.clone();
                let after = match declaration
                    .as_object_mut()
                    .map(|object| object.remove("after"))
                {
                    Some(Some(after)) => serde_json::from_value(after).map_err(|error| {
                        shared::incompatible(format!("{address}.after: {error}"))
                    })?,
                    Some(None) => Vec::new(),
                    None => {
                        return Err(shared::incompatible(format!("{address} must be an object")));
                    }
                };
                // Claims are the core's; each type reads only its own options.
                if let Some(object) = declaration.as_object_mut() {
                    object.remove("claims");
                }
                let publisher =
                    (registration.decode)(&declaration).map_err(|error| match error {
                        DeclarationError::Invalid(message) => {
                            DeclarationError::Invalid(format!("{address}: {message}"))
                        }
                        other => other,
                    })?;
                nodes.push(PublisherNode::new(address, publisher, after));
            }
        }
        Ok(nodes)
    }
}

impl Serialize for Publishers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Publishers {
    /// Reads every type by its current name; a declaration that uses both an earlier and the
    /// current name of one type is not one this CLI wrote.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let declared = BTreeMap::<String, BTreeMap<String, Value>>::deserialize(deserializer)?;
        let mut publishers = BTreeMap::new();
        for (kind, instances) in declared {
            let name =
                Registration::find(&kind).map_or(kind.as_str(), |registration| registration.name);
            if publishers.insert(name.to_owned(), instances).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "publishers.{name} is declared twice, also as publishers.{kind}"
                )));
            }
        }
        Ok(Self(publishers))
    }
}

/// Checks that no two publishers write the same remote object for `version`, with each claim's
/// key rendered for it.
pub fn check_claims(publishers: &Publishers, version: &Version) -> Result<(), String> {
    let mut claimed: BTreeMap<Claim, (String, String)> = BTreeMap::new();
    for (address, claim) in publishers.claims().map_err(|error| error.to_string())? {
        let template = claim.key.clone();
        let claim = Claim {
            key: shared::template::render(&claim.key, version),
            ..claim
        };
        if let Some((other, other_template)) = claimed.get(&claim) {
            let as_written = if *other_template == template {
                String::new()
            } else {
                format!(", as {other_template} and {template}")
            };
            return Err(format!(
                "publishers.{other} and publishers.{address} both claim {} in {} for release {version}{as_written}",
                claim.key, claim.destination
            ));
        }
        claimed.insert(claim, (address, template));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::ReleaseDeclaration;
    use serde_json::json;

    fn declaration(publishers: Value) -> String {
        json!({
            "schemaVersion": 0,
            "version": "0.1.0",
            "versionFiles": [],
            "tag": { "format": "v{version}" },
            "analyzer": { "convention": "gitmoji", "gitmoji": { "catalog": {} } },
            "changelog": { "path": "CHANGELOG.md" },
            "publishers": publishers
        })
        .to_string()
    }

    #[test]
    fn an_unknown_publisher_type_is_a_declaration_for_another_version() {
        let error =
            ReleaseDeclaration::from_json(&declaration(json!({ "npm": { "default": {} } })))
                .unwrap_err();
        assert!(
            matches!(error, DeclarationError::IncompatibleUnstableSchema(_)),
            "{error}"
        );
        assert!(error.to_string().contains("unknown publisher type \"npm\""));
    }

    #[test]
    fn claims_are_checked_with_the_released_version() {
        let declaration = ReleaseDeclaration::from_json(&declaration(json!({
            "oci-image": {
                "image": {
                    "repository": "ghcr.io/owner/project",
                    "platforms": {
                        "linux/amd64": "/nix/store/00000000000000000000000000000000-image.tar.gz"
                    },
                    "tags": ["v{version}"],
                    "aliases": ["0.1.0"],
                    "username": { "env": "REGISTRY_USERNAME" },
                    "password": { "env": "REGISTRY_TOKEN" },
                    "claims": [
                        { "destination": "oci://ghcr.io/owner/project", "key": "v{version}" },
                        { "destination": "oci://ghcr.io/owner/project", "key": "0.1.0" }
                    ]
                }
            },
            "oci-helm": {
                "chart": {
                    "repository": "ghcr.io/owner/project",
                    "chart": "/nix/store/00000000000000000000000000000000-chart.tgz",
                    "username": { "env": "REGISTRY_USERNAME" },
                    "password": { "env": "REGISTRY_TOKEN" },
                    "claims": [
                        { "destination": "oci://ghcr.io/owner/project", "key": "{version}" }
                    ]
                }
            }
        })))
        .unwrap();
        // The claims decode apart from each type's own options.
        assert_eq!(declaration.publishers.nodes().unwrap().len(), 2);

        // The sample version that Nix checks with does not collide.
        check_claims(&declaration.publishers, &Version::new(1, 2, 3)).unwrap();
        let error = check_claims(&declaration.publishers, &declaration.version).unwrap_err();
        assert!(
            error.contains("publishers.oci-image.image and publishers.oci-helm.chart"),
            "{error}"
        );
        assert!(
            error.ends_with(
                "both claim 0.1.0 in oci://ghcr.io/owner/project for release 0.1.0, as 0.1.0 and {version}"
            ),
            "{error}"
        );
    }

    #[test]
    fn only_credential_options_declare_secrets() {
        let declaration = ReleaseDeclaration::from_json(&declaration(json!({
            "github-release": {
                "default": {
                    "apiUrl": "https://api.github.com",
                    "repository": "owner/project",
                    "token": { "env": "GITHUB_TOKEN" },
                    // An asset named `env` has the shape of a secret source, but is none.
                    "assets": {
                        "env": "/nix/store/00000000000000000000000000000000-tool"
                    }
                }
            },
            "oci-image": {
                "image": {
                    "repository": "ghcr.io/owner/project",
                    "platforms": {
                        "linux/amd64": "/nix/store/00000000000000000000000000000000-image.tar.gz"
                    },
                    "tags": ["v{version}"],
                    "username": { "value": "oauth2accesstoken" },
                    "password": { "env": "REGISTRY_TOKEN" }
                }
            }
        })))
        .unwrap();
        assert_eq!(
            declaration.publishers.secrets(),
            [
                (
                    "oci-image.image.password".to_owned(),
                    SecretSource::Env("REGISTRY_TOKEN".into())
                ),
                (
                    "github-release.default.token".to_owned(),
                    SecretSource::Env("GITHUB_TOKEN".into())
                ),
            ]
        );
    }
}
