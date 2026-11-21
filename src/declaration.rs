use crate::analysis::Bump;
use crate::changelog::{
    ChangelogError, ChangelogHeading, DEFAULT_HEADING, DEFAULT_UNRELEASED_HEADING,
    validate_unreleased_heading,
};
use minijinja::{Environment, UndefinedBehavior, value::Serde};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub use crate::publishers::Publishers;
use std::error::Error;
use std::fmt;

/// The schema version of declarations before Presswerk 1.0. Its shape may change between
/// Presswerk versions, so a declaration that does not decode is treated as written for another
/// version instead of as invalid.
pub const UNSTABLE_SCHEMA_VERSION: i64 = 0;

pub const SCHEMA_VERSION: i64 = UNSTABLE_SCHEMA_VERSION;
pub const DEFAULT_CHANGELOG_TEMPLATE: &str = include_str!("release-notes.md.j2");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseDeclaration {
    pub schema_version: u32,
    pub version: Version,
    // Declarations created before this option, such as predecessor release commits, omit it.
    #[serde(default)]
    pub stable: bool,
    pub version_files: Vec<VersionFile>,
    pub tag: Tag,
    pub analyzer: Analyzer,
    pub changelog: Changelog,
    pub publishers: Publishers,
    /// The previous release to trust although this CLI cannot read its declaration. Only the
    /// CLI checks it against Git.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_predecessor: Option<DeclaredTrustedPredecessor>,
}

/// The `trustedPredecessor` option of the release module, as written into the declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredTrustedPredecessor {
    pub tag: String,
    pub commit: String,
}

impl ReleaseDeclaration {
    pub fn from_json(json: &str) -> Result<Self, DeclarationError> {
        let value: Value = serde_json::from_str(json).map_err(DeclarationError::InvalidJson)?;
        let schema_version = value
            .as_object()
            .and_then(|object| object.get("schemaVersion"))
            .ok_or(DeclarationError::MissingSchemaVersion)?
            .as_i64()
            .ok_or(DeclarationError::InvalidSchemaVersion)?;

        if schema_version != SCHEMA_VERSION {
            return Err(DeclarationError::UnsupportedSchemaVersion(schema_version));
        }

        let declaration: Self = serde_json::from_value(value).map_err(|error| {
            if schema_version == UNSTABLE_SCHEMA_VERSION {
                DeclarationError::IncompatibleUnstableSchema(error)
            } else {
                DeclarationError::InvalidDeclaration(error)
            }
        })?;
        declaration.validate()?;
        Ok(declaration)
    }

    pub fn validate(&self) -> Result<(), DeclarationError> {
        if i64::from(self.schema_version) != SCHEMA_VERSION {
            return Err(DeclarationError::UnsupportedSchemaVersion(i64::from(
                self.schema_version,
            )));
        }

        for version_file in &self.version_files {
            version_file.validate()?;
        }
        self.tag.validate()?;
        self.analyzer.validate()?;
        validate_relative_path(&self.changelog.path)?;
        self.changelog
            .heading()
            .and_then(|_| validate_unreleased_heading(&self.changelog.unreleased_heading))
            .map_err(|error| DeclarationError::Invalid(error.to_string()))?;
        validate_template(
            &self.changelog.template,
            "changelog.template",
            serde_json::json!({
                "version": "1.2.3",
                "unreleased": "Hand-written notes.",
                "groups": [{
                    "title": "Features",
                    "level": "minor",
                    "rank": 1,
                    "commits": [{
                        "text": "Add search",
                        "message": "Add search",
                        "scope": "cli",
                        "breaking": false,
                        "id": "0123456789abcdef"
                    }]
                }]
            }),
        )?;

        self.publishers.nodes().map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VersionFile {
    Toml(TomlVersionFile),
    Json(JsonVersionFile),
    Plain(PlainVersionFile),
    Regex(RegexVersionFile),
    CargoLock(CargoLockVersionFile),
    CargoWorkspace(CargoWorkspaceVersionFile),
    Badge(BadgeVersionFile),
}

impl VersionFile {
    fn validate(&self) -> Result<(), DeclarationError> {
        match self {
            Self::Toml(file) => {
                validate_relative_path(&file.path)?;
                key_segments(&file.toml).map_err(|error| {
                    DeclarationError::Invalid(format!("TOML key selector {error}"))
                })?;
            }
            Self::Json(file) => {
                validate_relative_path(&file.path)?;
                key_segments(&file.json).map_err(|error| {
                    DeclarationError::Invalid(format!("JSON key selector {error}"))
                })?;
            }
            Self::Plain(file) => validate_relative_path(&file.path)?,
            Self::Regex(file) => {
                validate_relative_path(&file.path)?;
                file.compile()?;
            }
            Self::CargoLock(file) => {
                validate_relative_path(&file.path)?;
                if file.cargo_lock.is_empty() {
                    return Err(DeclarationError::Invalid(
                        "Cargo.lock package name must not be empty".into(),
                    ));
                }
            }
            Self::CargoWorkspace(file) => {
                validate_relative_path(&file.path)?;
                validate_relative_path(&file.cargo_workspace)?;
            }
            Self::Badge(file) => validate_relative_path(&file.path)?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TomlVersionFile {
    pub path: String,
    pub toml: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonVersionFile {
    pub path: String,
    pub json: String,
}

/// The segments of a TOML dotted key, which also selects JSON keys. The flake parses keys as TOML,
/// too.
pub fn key_segments(key: &str) -> Result<Vec<String>, String> {
    toml_edit::Key::parse(key)
        .map(|keys| keys.iter().map(|key| key.get().to_owned()).collect())
        .map_err(|error| format!("{key:?} is not a TOML dotted key: {}", error.message()))
}

/// A file that contains nothing but the version, optionally followed by a line break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlainVersionFile {
    pub path: String,
    pub plain: PlainOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlainOptions {}

/// A version found by a pattern with a capture group named `version` that matches exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegexVersionFile {
    pub path: String,
    pub regex: String,
}

impl RegexVersionFile {
    /// Compile the pattern with `^` and `$` matching at line boundaries.
    pub fn compile(&self) -> Result<regex::Regex, DeclarationError> {
        let pattern = regex::RegexBuilder::new(&self.regex)
            .multi_line(true)
            .build()
            .map_err(|error| {
                DeclarationError::Invalid(format!("invalid pattern for {}: {error}", self.path))
            })?;
        if !pattern
            .capture_names()
            .flatten()
            .any(|name| name == "version")
        {
            return Err(DeclarationError::Invalid(format!(
                "pattern for {} must have a capture group named version",
                self.path
            )));
        }
        Ok(pattern)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoLockVersionFile {
    pub path: String,
    pub cargo_lock: String,
}

/// The `Cargo.lock` of a Cargo workspace whose manifest `cargo_workspace` declares the members.
/// Every member that inherits `workspace.package.version` has its package entry written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoWorkspaceVersionFile {
    pub path: String,
    pub cargo_workspace: String,
}

/// A version badge, generated from scratch on every release and never read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeVersionFile {
    pub path: String,
    pub badge: BadgeOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BadgeOptions {
    pub color_scheme: ColorScheme,
}

/// The page colour scheme a badge is made for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    /// Follow the viewer's system colour scheme.
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub format: String,
}

const VERSION_PLACEHOLDER: &str = "{version}";

impl Tag {
    fn validate(&self) -> Result<(), DeclarationError> {
        self.expression()
            .map(|_| ())
            .map_err(|error| DeclarationError::Invalid(error.to_string()))
    }

    pub fn render(&self, version: &Version) -> Result<String, TagError> {
        self.expression()?;
        Ok(self
            .format
            .replacen(VERSION_PLACEHOLDER, &version.to_string(), 1))
    }

    pub fn parse(&self, tag: &str) -> Result<Option<Version>, TagError> {
        let (start, end) = self.expression()?;
        let Some(version) = tag
            .strip_prefix(&self.format[..start])
            .and_then(|tag| tag.strip_suffix(&self.format[end..]))
        else {
            return Ok(None);
        };

        Ok(Version::parse(version).ok())
    }

    fn expression(&self) -> Result<(usize, usize), TagError> {
        let start = self.format.find(VERSION_PLACEHOLDER).ok_or(TagError)?;
        let end = start + VERSION_PLACEHOLDER.len();
        if self.format[end..].contains(VERSION_PLACEHOLDER)
            || self.format[..start].ends_with('{')
            || self.format[end..].starts_with('}')
        {
            return Err(TagError);
        }
        Ok((start, end))
    }
}

#[derive(Debug)]
pub struct TagError;

impl fmt::Display for TagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "tag.format must contain {version} exactly once, not adjacent to another brace",
        )
    }
}

impl Error for TagError {}

/// The one analyzer a release declares, named by its `convention` next to its settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "convention", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Analyzer {
    Gitmoji { gitmoji: GitmojiAnalyzer },
    Conventional { conventional: ConventionalAnalyzer },
}

impl Analyzer {
    fn validate(&self) -> Result<(), DeclarationError> {
        match self {
            Self::Gitmoji { gitmoji } => gitmoji.validate(),
            Self::Conventional { conventional } => conventional.validate(),
        }
    }
}

impl GitmojiAnalyzer {
    fn validate(&self) -> Result<(), DeclarationError> {
        let mut emojis = HashSet::new();
        for (shortcode, entry) in &self.catalog {
            if shortcode.is_empty() || shortcode.contains(':') {
                return Err(DeclarationError::Invalid(format!(
                    "Gitmoji catalog key {shortcode:?} must be a shortcode without surrounding colons"
                )));
            }
            if entry.emoji.is_empty() {
                return Err(DeclarationError::Invalid(format!(
                    "Gitmoji catalog entry {shortcode:?} must have a non-empty emoji"
                )));
            }
            if entry.description.is_empty() {
                return Err(DeclarationError::Invalid(format!(
                    "Gitmoji catalog entry {shortcode:?} must have a non-empty description"
                )));
            }
            if !emojis.insert(strip_variation_selectors(&entry.emoji)) {
                return Err(DeclarationError::Invalid(format!(
                    "Gitmoji catalog emoji {:?} is not unique after removing variation selectors",
                    entry.emoji
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConventionalAnalyzer {
    pub catalog: ConventionalCatalog,
}

impl ConventionalAnalyzer {
    fn validate(&self) -> Result<(), DeclarationError> {
        for (name, entry) in &self.catalog {
            if !is_conventional_type(name) {
                return Err(DeclarationError::Invalid(format!(
                    "Conventional Commits type {name:?} must consist of lowercase letters, digits, hyphens, or underscores"
                )));
            }
            if entry.title.is_empty() {
                return Err(DeclarationError::Invalid(format!(
                    "Conventional Commits type {name:?} must have a non-empty title"
                )));
            }
        }
        Ok(())
    }
}

fn is_conventional_type(name: &str) -> bool {
    name.starts_with(|character: char| character.is_ascii_lowercase() || character.is_ascii_digit())
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

/// Conventional Commits types keyed by their lowercase name.
pub type ConventionalCatalog = BTreeMap<String, ConventionalCatalogEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConventionalCatalogEntry {
    pub title: String,
    #[serde(deserialize_with = "deserialize_semver")]
    pub semver: Option<Bump>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitmojiAnalyzer {
    pub catalog: GitmojiCatalog,
}

pub type GitmojiCatalog = BTreeMap<String, GitmojiCatalogEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitmojiCatalogEntry {
    pub emoji: String,
    pub description: String,
    #[serde(deserialize_with = "deserialize_semver")]
    pub semver: Option<Bump>,
}

fn deserialize_semver<'de, D>(deserializer: D) -> Result<Option<Bump>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Changelog {
    pub path: String,
    #[serde(default = "default_changelog_template")]
    pub template: String,
    #[serde(default = "default_changelog_heading")]
    pub heading: String,
    #[serde(default = "default_changelog_unreleased_heading")]
    pub unreleased_heading: String,
}

impl Changelog {
    /// The declared version heading; validation has already accepted its format.
    pub fn heading(&self) -> Result<ChangelogHeading, ChangelogError> {
        ChangelogHeading::parse(&self.heading)
    }
}

fn default_changelog_heading() -> String {
    DEFAULT_HEADING.into()
}

fn default_changelog_unreleased_heading() -> String {
    DEFAULT_UNRELEASED_HEADING.into()
}

fn default_changelog_template() -> String {
    DEFAULT_CHANGELOG_TEMPLATE.into()
}

#[derive(Debug)]
pub enum DeclarationError {
    InvalidJson(serde_json::Error),
    MissingSchemaVersion,
    InvalidSchemaVersion,
    UnsupportedSchemaVersion(i64),
    IncompatibleUnstableSchema(serde_json::Error),
    InvalidDeclaration(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid declaration JSON: {error}"),
            Self::MissingSchemaVersion => write!(formatter, "declaration is missing schemaVersion"),
            Self::InvalidSchemaVersion => {
                write!(formatter, "declaration schemaVersion must be an integer")
            }
            Self::UnsupportedSchemaVersion(version) => write!(
                formatter,
                "unsupported declaration schemaVersion {version}; update the locked `presswerk` flake input so the release module and CLI use the same Presswerk version"
            ),
            Self::IncompatibleUnstableSchema(error) => write!(
                formatter,
                "declaration with the unstable schemaVersion {UNSTABLE_SCHEMA_VERSION} does not match this CLI ({error}); update the locked `presswerk` flake input so the release module and CLI use the same Presswerk version"
            ),
            Self::InvalidDeclaration(error) => write!(formatter, "invalid declaration: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl Error for DeclarationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error)
            | Self::IncompatibleUnstableSchema(error)
            | Self::InvalidDeclaration(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn strip_variation_selectors(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\u{fe0f}')
        .collect()
}

pub(crate) fn validate_template(
    template: &str,
    option: &str,
    context: Value,
) -> Result<(), DeclarationError> {
    let mut environment = Environment::new();
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    environment
        .render_str(template, Serde(&context))
        .map(|_| ())
        .map_err(|error| {
            DeclarationError::Invalid(format!("invalid MiniJinja template in {option}: {error}"))
        })
}

fn validate_relative_path(path: &str) -> Result<(), DeclarationError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(DeclarationError::Invalid(format!(
            "path {path:?} must be a repository-relative path"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publishers::github_release::{
        DEFAULT_GITHUB_RELEASE_BODY, GithubReleaseDeclaration,
    };
    use crate::publishers::oci::helm::OciHelmDeclaration;
    use serde_json::json;

    /// The declared instance `name` of `kind`, decoded into its type's shape without `after`.
    fn instance<T: serde::de::DeserializeOwned>(
        declaration: &ReleaseDeclaration,
        kind: &str,
        name: &str,
    ) -> T {
        let mut value = declaration.publishers.get(kind, name).unwrap().clone();
        value.as_object_mut().unwrap().remove("after");
        serde_json::from_value(value).unwrap()
    }

    fn after(declaration: &ReleaseDeclaration, address: &str) -> Vec<String> {
        let nodes = declaration.publishers.nodes().unwrap();
        let node = nodes.iter().find(|node| node.name == address).unwrap();
        node.after.clone()
    }

    /// An OCI instance with the claims its Nix module derives: one per tag template.
    fn claimed(mut instance: Value) -> Value {
        let destination = format!("oci://{}", instance["repository"].as_str().unwrap());
        let keys: Vec<Value> = match instance.get("tags") {
            Some(tags) => tags
                .as_array()
                .unwrap()
                .iter()
                .chain(instance["aliases"].as_array().unwrap())
                .cloned()
                .collect(),
            None => vec![json!("{version}")],
        };
        instance["claims"] = keys
            .into_iter()
            .map(|key| json!({ "destination": destination, "key": key }))
            .collect();
        instance
    }

    fn github(declaration: &ReleaseDeclaration) -> GithubReleaseDeclaration {
        instance(declaration, "github-release", "default")
    }

    fn declaration(patch: impl FnOnce(&mut Value)) -> String {
        let mut declaration = json!({
            "schemaVersion": 0,
            "version": "1.2.3",
            "stable": true,
            "versionFiles": [
                { "path": "Cargo.toml", "toml": "package.version" },
                { "path": "Cargo.lock", "cargoLock": "presswerk" }
            ],
            "tag": { "format": "v{version}" },
            "analyzer": {
                "convention": "gitmoji",
                "gitmoji": {
                    "catalog": {
                        "sparkles": {
                            "emoji": "✨",
                            "description": "Introduce new features.",
                            "semver": "minor"
                        }
                    }
                }
            },
            "changelog": { "path": "CHANGELOG.md" },
            "publishers": {
                "github-release": {
                    "default": {
                        "apiUrl": "https://api.github.com",
                        "repository": "jkuball/presswerk",
                        "token": { "env": "GITHUB_TOKEN" }
                    }
                }
            }
        });
        patch(&mut declaration);
        declaration.to_string()
    }

    #[test]
    fn parses_schema_zero_with_the_declared_gitmoji_catalog() {
        let declaration = ReleaseDeclaration::from_json(&declaration(|_| {})).unwrap();
        assert_eq!(declaration.schema_version, 0);
        assert_eq!(declaration.version, Version::new(1, 2, 3));
        assert!(declaration.stable);
        assert_eq!(
            declaration.changelog,
            Changelog {
                path: "CHANGELOG.md".into(),
                template: DEFAULT_CHANGELOG_TEMPLATE.into(),
                heading: DEFAULT_HEADING.into(),
                unreleased_heading: DEFAULT_UNRELEASED_HEADING.into(),
            }
        );
        assert_eq!(github(&declaration).api_url, "https://api.github.com");
        assert_eq!(github(&declaration).body, DEFAULT_GITHUB_RELEASE_BODY);
        assert!(after(&declaration, "github-release.default").is_empty());
        let Analyzer::Gitmoji { gitmoji } = &declaration.analyzer else {
            panic!("expected the Gitmoji analyzer");
        };
        assert_eq!(gitmoji.catalog["sparkles"].semver, Some(Bump::Minor));
    }

    #[test]
    fn old_declarations_default_to_initial_development() {
        let legacy = declaration(|value| {
            value["version"] = json!("0.18.0");
            value.as_object_mut().unwrap().remove("stable");
        });
        assert!(!ReleaseDeclaration::from_json(&legacy).unwrap().stable);
    }

    fn conventional(value: &mut Value) {
        value["analyzer"] = json!({
            "convention": "conventional",
            "conventional": {
                "catalog": {
                    "feat": { "title": "Features", "semver": "minor" },
                    "docs": { "title": "Documentation", "semver": null }
                }
            }
        });
    }

    #[test]
    fn parses_the_conventional_commits_analyzer() {
        let declaration = ReleaseDeclaration::from_json(&declaration(conventional)).unwrap();
        let Analyzer::Conventional { conventional } = &declaration.analyzer else {
            panic!("expected the Conventional Commits analyzer");
        };
        assert_eq!(conventional.catalog["feat"].semver, Some(Bump::Minor));
        assert_eq!(conventional.catalog["docs"].semver, None);
        assert_eq!(conventional.catalog["docs"].title, "Documentation");
    }

    #[test]
    fn declares_exactly_one_analyzer_matching_its_convention() {
        for patch in [
            |value: &mut Value| {
                conventional(value);
                value["analyzer"]["gitmoji"] = json!({ "catalog": {} });
            },
            |value: &mut Value| {
                conventional(value);
                value["analyzer"]["convention"] = json!("gitmoji");
            },
            |value: &mut Value| {
                value["analyzer"].as_object_mut().unwrap().remove("gitmoji");
            },
            |value: &mut Value| {
                value["analyzer"]["convention"] = json!("angular");
            },
        ] {
            assert!(matches!(
                ReleaseDeclaration::from_json(&declaration(patch)).unwrap_err(),
                DeclarationError::IncompatibleUnstableSchema(_)
            ));
        }
    }

    #[test]
    fn conventional_types_are_lowercase_names_with_titles() {
        for (name, title, message) in [
            ("Feat", "Features", "lowercase letters"),
            ("feat(cli)", "Features", "lowercase letters"),
            ("", "Features", "lowercase letters"),
            ("feat", "", "non-empty title"),
        ] {
            let invalid = declaration(|value| {
                conventional(value);
                value["analyzer"]["conventional"]["catalog"] =
                    json!({ name: { "title": title, "semver": null } });
            });
            assert!(
                ReleaseDeclaration::from_json(&invalid)
                    .unwrap_err()
                    .to_string()
                    .contains(message),
                "{name:?}"
            );
        }
    }

    #[test]
    fn requires_each_catalog_entry_to_declare_a_semver_level() {
        let missing = declaration(|value| {
            value["analyzer"]["gitmoji"]["catalog"]["sparkles"]
                .as_object_mut()
                .unwrap()
                .remove("semver");
        });

        assert!(matches!(
            ReleaseDeclaration::from_json(&missing).unwrap_err(),
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn rejects_duplicate_catalog_emojis_without_variation_selectors() {
        let duplicate = declaration(|value| {
            value["analyzer"]["gitmoji"]["catalog"] = json!({
                "zap": {
                    "emoji": "⚡️",
                    "description": "Improve performance.",
                    "semver": "patch"
                },
                "custom_zap": {
                    "emoji": "⚡",
                    "description": "Duplicate the zap intention.",
                    "semver": null
                }
            });
        });

        assert!(
            ReleaseDeclaration::from_json(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("not unique")
        );
    }

    #[test]
    fn treats_an_undecodable_unstable_schema_as_written_for_another_version() {
        let unexpected = declaration(|value| value["unexpected"] = json!(true));
        let error = ReleaseDeclaration::from_json(&unexpected).unwrap_err();

        assert!(error.to_string().contains("locked `presswerk` flake input"));
        assert!(matches!(
            error,
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn rejects_a_missing_changelog() {
        let missing = declaration(|value| {
            value.as_object_mut().unwrap().remove("changelog");
        });
        assert!(matches!(
            ReleaseDeclaration::from_json(&missing).unwrap_err(),
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn rejects_an_unsupported_schema_before_the_remainder() {
        let error = ReleaseDeclaration::from_json(
            r#"{ "schemaVersion": 1, "version": false, "unexpected": true }"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("locked `presswerk` flake input"));
        assert!(matches!(
            error,
            DeclarationError::UnsupportedSchemaVersion(1)
        ));
    }

    #[test]
    fn accepts_a_release_without_publishers() {
        let empty = declaration(|value| value["publishers"]["github-release"] = json!({}));
        let declaration = ReleaseDeclaration::from_json(&empty).unwrap();
        assert!(declaration.publishers.is_empty());
        assert!(declaration.publishers.nodes().unwrap().is_empty());
    }

    #[test]
    fn decodes_publisher_dependencies() {
        let declaration = declaration(|value| {
            value["publishers"]["github-release"]["default"]["after"] = json!(["oci-image.api"]);
        });
        let decoded = ReleaseDeclaration::from_json(&declaration).unwrap();

        assert_eq!(after(&decoded, "github-release.default"), ["oci-image.api"]);
    }

    #[test]
    fn validates_repository_relative_paths() {
        let invalid = declaration(|value| {
            value["versionFiles"][0]["path"] = json!("../Cargo.toml");
        });
        assert!(
            ReleaseDeclaration::from_json(&invalid)
                .unwrap_err()
                .to_string()
                .contains("repository-relative")
        );
    }

    #[test]
    fn validates_generic_toml_selectors() {
        let generic = declaration(|value| {
            value["versionFiles"][0]["toml"] = json!("project.version");
        });
        ReleaseDeclaration::from_json(&generic).unwrap();

        let empty = declaration(|value| value["versionFiles"][0]["toml"] = json!(""));
        assert!(
            ReleaseDeclaration::from_json(&empty)
                .unwrap_err()
                .to_string()
                .contains("TOML key selector \"\" is not a TOML dotted key")
        );
    }

    #[test]
    fn parses_keys_as_toml_dotted_keys() {
        // The same inputs as the nix-unit test "test keys parse as TOML dotted keys".
        for (key, segments) in [
            ("version", &["version"][..]),
            (r#"packages."".version"#, &["packages", "", "version"]),
            (r#"packages."a.b".version"#, &["packages", "a.b", "version"]),
            ("a.'x.y'.b", &["a", "x.y", "b"]),
            (r#""q\"z""#, &["q\"z"]),
            (
                r#"dependencies."@scope/pkg".version"#,
                &["dependencies", "@scope/pkg", "version"],
            ),
            (" a . b ", &["a", "b"]),
            (r#""\u00e9""#, &["é"]),
        ] {
            assert_eq!(key_segments(key).unwrap(), segments, "{key:?}");
        }
        for key in [
            "", "a..b", "a.", "a = 1\nb", "a = 0 #", "@scope", "\"a", "a b", "[a]\nb", "a#",
        ] {
            assert!(
                key_segments(key)
                    .unwrap_err()
                    .contains("is not a TOML dotted key"),
                "{key:?}"
            );
        }

        let unbalanced = declaration(|value| {
            value["versionFiles"][0]["toml"] = json!(r#"packages."a.version"#);
        });
        assert!(
            ReleaseDeclaration::from_json(&unbalanced)
                .unwrap_err()
                .to_string()
                .contains(r#"TOML key selector "packages.\"a.version" is not a TOML dotted key"#)
        );
    }

    #[test]
    fn decodes_badge_version_files() {
        let badge = declaration(|value| {
            value["versionFiles"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "path": ".github/presswerk-badge.svg", "badge": { "colorScheme": "dark" } }));
        });
        let decoded = ReleaseDeclaration::from_json(&badge).unwrap();
        assert_eq!(
            decoded.version_files[2],
            VersionFile::Badge(BadgeVersionFile {
                path: ".github/presswerk-badge.svg".into(),
                badge: BadgeOptions {
                    color_scheme: ColorScheme::Dark,
                },
            })
        );

        let unknown = declaration(|value| {
            value["versionFiles"][0] = json!({ "path": "badge.svg", "badge": { "style": "flat" } });
        });
        assert!(ReleaseDeclaration::from_json(&unknown).is_err());
    }

    #[test]
    fn decodes_cargo_workspace_version_files() {
        let workspace = declaration(|value| {
            value["versionFiles"][1] =
                json!({ "path": "rust/Cargo.lock", "cargoWorkspace": "rust/Cargo.toml" });
        });
        assert_eq!(
            ReleaseDeclaration::from_json(&workspace)
                .unwrap()
                .version_files[1],
            VersionFile::CargoWorkspace(CargoWorkspaceVersionFile {
                path: "rust/Cargo.lock".into(),
                cargo_workspace: "rust/Cargo.toml".into(),
            })
        );

        let outside = declaration(|value| {
            value["versionFiles"][1] =
                json!({ "path": "Cargo.lock", "cargoWorkspace": "../Cargo.toml" });
        });
        assert!(ReleaseDeclaration::from_json(&outside).is_err());
    }

    #[test]
    fn regex_version_files_need_a_valid_pattern_with_a_version_group() {
        for (pattern, message) in [
            ("(?<version>", "invalid pattern"),
            (r"^version: (\S+)$", "capture group named version"),
        ] {
            let invalid = declaration(|value| {
                value["versionFiles"][0] = json!({ "path": "Chart.yaml", "regex": pattern });
            });
            assert!(
                ReleaseDeclaration::from_json(&invalid)
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
    }

    #[test]
    fn decodes_and_validates_github_release_assets() {
        let without = ReleaseDeclaration::from_json(&declaration(|_| {})).unwrap();
        assert!(github(&without).assets.is_empty());

        let with = declaration(|value| {
            value["publishers"]["github-release"]["default"]["assets"] =
                json!({ "tool-x86_64-linux": "/nix/store/abc-tool/bin/tool" });
        });
        let decoded = ReleaseDeclaration::from_json(&with).unwrap();
        assert_eq!(
            github(&decoded).assets["tool-x86_64-linux"],
            "/nix/store/abc-tool/bin/tool"
        );

        for (name, path, message) in [
            ("", "/nix/store/abc-tool", "must be non-empty"),
            ("a/b", "/nix/store/abc-tool", "contain no slash"),
            ("tool", "relative/tool", "absolute store path"),
        ] {
            let invalid = declaration(|value| {
                value["publishers"]["github-release"]["default"]["assets"] = json!({ name: path });
            });
            assert!(
                ReleaseDeclaration::from_json(&invalid)
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
    }

    #[test]
    fn validates_templates_with_their_documented_context() {
        let valid = declaration(|value| {
            value["changelog"]["template"] = json!(
                "{{ version }}{% for group in groups|sort(attribute=\"rank\") %}{{ group.title }}{{ group.level }}{{ group.rank }}{% for commit in group.commits %}{{ commit.text }}{{ commit.message }}{{ commit.scope }}{{ commit.breaking }}{{ commit.id }}{% endfor %}{% endfor %}"
            );
            value["publishers"]["github-release"]["default"]["body"] =
                json!("{{ notes }} {{ version }} {{ tag }}");
        });
        ReleaseDeclaration::from_json(&valid).unwrap();

        for (path, template) in [
            (
                &["changelog", "template"][..],
                "{{ groups[0].commits[0].unknown }}",
            ),
            (
                &["publishers", "github-release", "default", "body"][..],
                "{{ unknown }}",
            ),
        ] {
            let invalid = declaration(|value| {
                let mut field = value;
                for part in &path[..path.len() - 1] {
                    field = &mut field[*part];
                }
                field[path[path.len() - 1]] = json!(template);
            });
            let error = ReleaseDeclaration::from_json(&invalid).unwrap_err();
            assert!(error.to_string().contains("MiniJinja template"), "{error}");
        }
    }

    #[test]
    fn validates_the_changelog_heading() {
        let declared = declaration(|value| {
            value["changelog"]["heading"] = json!("## v{version} ({date})");
        });
        assert_eq!(
            ReleaseDeclaration::from_json(&declared)
                .unwrap()
                .changelog
                .heading,
            "## v{version} ({date})"
        );

        let invalid = declaration(|value| value["changelog"]["heading"] = json!("v{version}"));
        assert!(
            ReleaseDeclaration::from_json(&invalid)
                .unwrap_err()
                .to_string()
                .contains("changelog.heading")
        );

        let invalid = declaration(|value| {
            value["changelog"]["unreleasedHeading"] = json!("Unreleased");
        });
        assert!(
            ReleaseDeclaration::from_json(&invalid)
                .unwrap_err()
                .to_string()
                .contains("changelog.unreleasedHeading")
        );
    }

    #[test]
    fn validates_the_tag_version_placeholder() {
        for format in [
            "v{version}",
            "release/{version}/source",
            "{release}-{version}-{source}",
        ] {
            let declaration = declaration(|value| value["tag"]["format"] = json!(format));
            ReleaseDeclaration::from_json(&declaration).unwrap();
        }

        for format in [
            "release",
            "{version}-{version}",
            "v{{ version }}",
            "v{{version}}",
            "v{%raw%}{{version}}{%endraw%}",
            "v{{version}",
            "v{version}}",
        ] {
            let declaration = declaration(|value| value["tag"]["format"] = json!(format));
            assert!(
                ReleaseDeclaration::from_json(&declaration)
                    .unwrap_err()
                    .to_string()
                    .contains("exactly once")
            );
        }
    }

    #[test]
    fn renders_and_parses_release_tags() {
        let version = Version::new(1, 2, 3);
        for format in [
            "v{version}",
            "release/{version}/source",
            "{release}-{version}-{source}",
        ] {
            let tag = Tag {
                format: format.into(),
            };
            let rendered = tag.render(&version).unwrap();
            assert_eq!(tag.parse(&rendered).unwrap(), Some(version.clone()));
        }

        let tag = Tag {
            format: "release/{version}/source".into(),
        };
        assert_eq!(tag.render(&version).unwrap(), "release/1.2.3/source");
        assert_eq!(tag.parse("release/next/source").unwrap(), None);
        assert_eq!(tag.parse("v1.2.3").unwrap(), None);
    }

    #[test]
    fn validates_github_api_urls() {
        let enterprise = declaration(|value| {
            value["publishers"]["github-release"]["default"]["apiUrl"] =
                json!("https://github.example/api/v3");
        });
        ReleaseDeclaration::from_json(&enterprise).unwrap();

        for api_url in [
            "github.example",
            "file:///tmp/github",
            "https://api.github.com?token=secret",
            "https://api.github.com#fragment",
        ] {
            let invalid = declaration(|value| {
                value["publishers"]["github-release"]["default"]["apiUrl"] = json!(api_url);
            });
            assert!(
                ReleaseDeclaration::from_json(&invalid)
                    .unwrap_err()
                    .to_string()
                    .contains("GitHub API URL")
            );
        }
    }

    #[test]
    fn accepts_logical_secret_names_and_rejects_empty_names() {
        let logical_name = declaration(|value| {
            value["publishers"]["github-release"]["default"]["token"]["env"] =
                json!("github-token");
        });
        ReleaseDeclaration::from_json(&logical_name).unwrap();

        let literal = declaration(|value| {
            value["publishers"]["github-release"]["default"]["token"] = json!({ "value": "ghp_x" });
        });
        assert!(
            ReleaseDeclaration::from_json(&literal)
                .unwrap_err()
                .to_string()
                .contains("the GitHub token must be a secret")
        );

        let empty_name = declaration(|value| {
            value["publishers"]["github-release"]["default"]["token"]["env"] = json!("");
        });
        assert!(
            ReleaseDeclaration::from_json(&empty_name)
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn rejects_invalid_semantic_versions() {
        let invalid = declaration(|value| value["version"] = json!("next"));
        assert!(matches!(
            ReleaseDeclaration::from_json(&invalid).unwrap_err(),
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn decodes_and_validates_oci_helm_publishers() {
        let with = |repository: &str, chart: &str| {
            declaration(|value| {
                value["publishers"]["oci-helm"] = json!({
                    "default": {
                        "repository": repository,
                        "chart": chart,
                        "public": true,
                        "username": { "value": "oauth2accesstoken" },
                        "password": { "env": "REGISTRY_TOKEN" }
                    }
                });
            })
        };
        let decoded = ReleaseDeclaration::from_json(&with(
            "ghcr.io/owner/charts/project",
            "/nix/store/abc-project-1.2.3.tgz",
        ))
        .unwrap();
        let publisher: OciHelmDeclaration = instance(&decoded, "oci-helm", "default");
        assert!(after(&decoded, "oci-helm.default").is_empty());
        assert!(publisher.public);
        assert_eq!(publisher.chart, "/nix/store/abc-project-1.2.3.tgz");

        for (repository, chart, expected) in [
            (
                "owner/charts/project",
                "/nix/store/abc-project.tgz",
                "must start with a registry host",
            ),
            (
                "ghcr.io/owner/charts/project",
                "result/project.tgz",
                "absolute store path",
            ),
        ] {
            let error = ReleaseDeclaration::from_json(&with(repository, chart))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn legacy_oci_declarations_decode_but_mixed_names_do_not() {
        let publishers = json!({
            "default": {
                "repository": "ghcr.io/owner/project",
                "platforms": { "linux/amd64": "/nix/store/abc-image.tar.gz" },
                "tags": ["{version}"],
                "aliases": [],
                "username": { "env": "REGISTRY_USERNAME" },
                "password": { "env": "REGISTRY_TOKEN" }
            }
        });
        let legacy = declaration(|value| value["publishers"]["oci"] = publishers.clone());
        let decoded = ReleaseDeclaration::from_json(&legacy).unwrap();
        assert!(after(&decoded, "oci-image.default").is_empty());
        let normalized = serde_json::to_value(decoded).unwrap();
        assert!(normalized["publishers"].get("oci").is_none());
        assert!(normalized["publishers"].get("oci-image").is_some());

        let mixed = declaration(|value| {
            value["publishers"]["oci"] = publishers.clone();
            value["publishers"]["oci-image"] = publishers;
        });
        assert!(matches!(
            ReleaseDeclaration::from_json(&mixed).unwrap_err(),
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn legacy_helm_declarations_do_not_decode() {
        let declaration = declaration(|value| value["publishers"]["helm"] = json!({}));
        assert!(matches!(
            ReleaseDeclaration::from_json(&declaration).unwrap_err(),
            DeclarationError::IncompatibleUnstableSchema(_)
        ));
    }

    #[test]
    fn oci_helm_and_oci_image_publishers_do_not_share_the_release_tag() {
        let json = declaration(|value| {
            value["publishers"]["oci-helm"] = json!({
                "default": claimed(json!({
                    "repository": "ghcr.io/owner/charts/project",
                    "chart": "/nix/store/abc-project.tgz",
                    "username": { "env": "REGISTRY_USERNAME" },
                    "password": { "env": "REGISTRY_TOKEN" }
                }))
            });
            value["publishers"]["oci-image"] = json!({
                "default": claimed(json!({
                    "repository": "ghcr.io/owner/charts/project",
                    "platforms": { "linux/amd64": "/nix/store/abc-image.tar.gz" },
                    "tags": ["{version}"],
                    "aliases": [],
                    "username": { "env": "REGISTRY_USERNAME" },
                    "password": { "env": "REGISTRY_TOKEN" }
                }))
            });
        });

        // Nix rejects this with the sample version already; the CLI checks the released one.
        let declaration = ReleaseDeclaration::from_json(&json).unwrap();
        let error = crate::publishers::check_claims(&declaration.publishers, &declaration.version)
            .unwrap_err();
        assert!(
            error.contains("publishers.oci-image.default and publishers.oci-helm.default"),
            "{error}"
        );
        assert!(error.contains("both claim 1.2.3"), "{error}");
    }

    #[test]
    fn oci_image_publishers_of_one_repository_share_no_image_tag() {
        let oci = |repository: &str, tags: Value, aliases: Value| {
            claimed(json!({
                "repository": repository,
                "platforms": { "linux/amd64": "/nix/store/00000000000000000000000000000000-image.tar.gz" },
                "tags": tags,
                "aliases": aliases,
                "username": { "env": "REGISTRY_USERNAME" },
                "password": { "env": "REGISTRY_TOKEN" }
            }))
        };
        let with = |debug: Value| {
            let default = oci(
                "ghcr.io/owner/project",
                json!(["{version}"]),
                json!(["latest"]),
            );
            declaration(|value| {
                value["publishers"]["oci-image"] = json!({ "default": default, "debug": debug });
            })
        };
        // Nix rejects these with the sample version already; the CLI checks the released one.
        let claims = |json: String| {
            let declaration = ReleaseDeclaration::from_json(&json).unwrap();
            crate::publishers::check_claims(&declaration.publishers, &declaration.version)
        };
        assert!(
            claims(with(oci(
                "ghcr.io/owner/project",
                json!(["{version}-debug"]),
                json!(["debug"])
            )))
            .is_ok()
        );
        assert!(
            claims(with(oci(
                "ghcr.io/owner/other",
                json!(["{version}"]),
                json!(["latest"])
            )))
            .is_ok()
        );
        let error = claims(with(oci(
            "ghcr.io/owner/project",
            json!(["{version}-debug"]),
            json!(["latest"]),
        )))
        .unwrap_err();
        assert!(
            error.contains(
                "publishers.oci-image.debug and publishers.oci-image.default both claim latest in oci://ghcr.io/owner/project"
            ) && error.contains("both claim latest"),
            "{error}"
        );
    }
}
