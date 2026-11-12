use crate::analysis::Bump;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use url::Url;

/// The schema version of declarations before Presswerk 1.0. Its shape may change between
/// Presswerk versions, so a declaration that does not decode is treated as written for another
/// version instead of as invalid.
pub const UNSTABLE_SCHEMA_VERSION: i64 = 0;

pub const SCHEMA_VERSION: i64 = UNSTABLE_SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseDeclaration {
    pub schema_version: u32,
    pub version: Version,
    pub version_files: Vec<VersionFile>,
    pub tag: Tag,
    pub analyzer: Analyzer,
    pub changelog: Changelog,
    pub publishers: Publishers,
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

        // M1 only supports GitHub Releases, so every release needs one.
        if self.publishers.github_release.is_empty() {
            return Err(DeclarationError::Invalid(
                "at least one GitHub Release publisher is required".into(),
            ));
        }
        for (name, publisher) in &self.publishers.github_release {
            if name.is_empty() {
                return Err(DeclarationError::Invalid(
                    "GitHub Release publisher names must not be empty".into(),
                ));
            }
            publisher.validate()?;
        }
        for (name, publisher) in &self.publishers.oci {
            if name.is_empty() {
                return Err(DeclarationError::Invalid(
                    "OCI publisher names must not be empty".into(),
                ));
            }
            publisher
                .validate()
                .map_err(|error| DeclarationError::Invalid(format!("oci.{name}: {error}")))?;
        }

        Ok(())
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
    Badge(BadgeVersionFile),
}

impl VersionFile {
    fn validate(&self) -> Result<(), DeclarationError> {
        match self {
            Self::Toml(file) => {
                validate_relative_path(&file.path)?;
                if file.toml.is_empty() {
                    return Err(DeclarationError::Invalid(
                        "TOML key selector must not be empty".into(),
                    ));
                }
            }
            Self::Json(file) => {
                validate_relative_path(&file.path)?;
                if file.json.is_empty() {
                    return Err(DeclarationError::Invalid(
                        "JSON key selector must not be empty".into(),
                    ));
                }
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
#[serde(deny_unknown_fields)]
pub struct Changelog {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publishers {
    #[serde(rename = "github-release")]
    pub github_release: BTreeMap<String, GithubReleasePublisher>,
    /// Missing in declarations written before the OCI publisher existed.
    #[serde(default)]
    pub oci: BTreeMap<String, OciPublisher>,
}

/// Container images pushed to an OCI registry under tags rendered from the version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciPublisher {
    /// Registry host and repository path, such as `ghcr.io/owner/project`.
    pub repository: String,
    /// One image per platform, such as `linux/amd64`, each a path in the Nix store.
    pub images: BTreeMap<String, String>,
    /// Tag templates; `{version}` is replaced by the release version.
    pub tags: Vec<String>,
    pub username: Token,
    pub password: Token,
}

impl OciPublisher {
    fn validate(&self) -> Result<(), String> {
        crate::oci::RepositoryReference::parse(&self.repository)?;
        if self.images.is_empty() {
            return Err("an OCI publisher needs at least one image".into());
        }
        for (platform, path) in &self.images {
            crate::oci::Platform::parse(platform)?;
            if !path.starts_with('/') {
                return Err(format!(
                    "image {platform} must be an absolute store path, not {path:?}"
                ));
            }
        }
        crate::oci::validate_tag_templates(&self.tags)?;
        self.username
            .validate()
            .map_err(|error| error.to_string())?;
        self.password.validate().map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubReleasePublisher {
    pub api_url: String,
    pub repository: String,
    pub token: Token,
    /// Files attached to the release, by asset name. Each is a path in the Nix store.
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

impl GithubReleasePublisher {
    fn validate(&self) -> Result<(), DeclarationError> {
        let mut repository_parts = self.repository.split('/');
        if repository_parts.next().is_none_or(str::is_empty)
            || repository_parts.next().is_none_or(str::is_empty)
            || repository_parts.next().is_some()
        {
            return Err(DeclarationError::Invalid(format!(
                "GitHub repository {:?} must use owner/name form",
                self.repository
            )));
        }
        let api_url = Url::parse(&self.api_url).map_err(|error| {
            DeclarationError::Invalid(format!(
                "invalid GitHub API URL {:?}: {error}",
                self.api_url
            ))
        })?;
        if !matches!(api_url.scheme(), "http" | "https")
            || api_url.cannot_be_a_base()
            || api_url.query().is_some()
            || api_url.fragment().is_some()
        {
            return Err(DeclarationError::Invalid(format!(
                "GitHub API URL {:?} must be an HTTP(S) base URL",
                self.api_url
            )));
        }
        for (name, path) in &self.assets {
            if name.is_empty() || name.contains('/') {
                return Err(DeclarationError::Invalid(format!(
                    "GitHub Release asset name {name:?} must be non-empty and contain no slash"
                )));
            }
            if !path.starts_with('/') {
                return Err(DeclarationError::Invalid(format!(
                    "GitHub Release asset {name} must be an absolute store path, not {path:?}"
                )));
            }
        }
        self.token.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    pub secret: String,
}

impl Token {
    fn validate(&self) -> Result<(), DeclarationError> {
        if self.secret.is_empty() {
            return Err(DeclarationError::Invalid(
                "secret name must not be empty".into(),
            ));
        }
        Ok(())
    }
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
    use serde_json::json;

    fn declaration(patch: impl FnOnce(&mut Value)) -> String {
        let mut declaration = json!({
            "schemaVersion": 0,
            "version": "1.2.3",
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
                        "token": { "secret": "GITHUB_TOKEN" }
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
        assert_eq!(
            declaration.changelog,
            Changelog {
                path: "CHANGELOG.md".into()
            }
        );
        assert_eq!(
            declaration.publishers.github_release["default"].api_url,
            "https://api.github.com"
        );
        let Analyzer::Gitmoji { gitmoji } = &declaration.analyzer else {
            panic!("expected the Gitmoji analyzer");
        };
        assert_eq!(gitmoji.catalog["sparkles"].semver, Some(Bump::Minor));
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
    fn requires_a_github_release_publisher() {
        let empty = declaration(|value| value["publishers"]["github-release"] = json!({}));
        assert!(
            ReleaseDeclaration::from_json(&empty)
                .unwrap_err()
                .to_string()
                .contains("at least one")
        );
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
                .contains("must not be empty")
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
        assert!(
            without.publishers.github_release["default"]
                .assets
                .is_empty()
        );

        let with = declaration(|value| {
            value["publishers"]["github-release"]["default"]["assets"] =
                json!({ "tool-x86_64-linux": "/nix/store/abc-tool/bin/tool" });
        });
        let decoded = ReleaseDeclaration::from_json(&with).unwrap();
        assert_eq!(
            decoded.publishers.github_release["default"].assets["tool-x86_64-linux"],
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
            value["publishers"]["github-release"]["default"]["token"]["secret"] =
                json!("github-token");
        });
        ReleaseDeclaration::from_json(&logical_name).unwrap();

        let empty_name = declaration(|value| {
            value["publishers"]["github-release"]["default"]["token"]["secret"] = json!("");
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
}
