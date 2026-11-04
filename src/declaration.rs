use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const SCHEMA_VERSION: i64 = 1;

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

        let declaration: Self =
            serde_json::from_value(value).map_err(DeclarationError::InvalidDeclaration)?;
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
        validate_relative_path(&self.changelog.path)?;

        for (name, publisher) in &self.publishers.github_release {
            if name.is_empty() {
                return Err(DeclarationError::Invalid(
                    "GitHub Release publisher names must not be empty".into(),
                ));
            }
            publisher.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VersionFile {
    Toml(TomlVersionFile),
    CargoLock(CargoLockVersionFile),
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
            Self::CargoLock(file) => {
                validate_relative_path(&file.path)?;
                if file.cargo_lock.is_empty() {
                    return Err(DeclarationError::Invalid(
                        "Cargo.lock package name must not be empty".into(),
                    ));
                }
            }
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoLockVersionFile {
    pub path: String,
    pub cargo_lock: String,
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
        if self.format[end..].contains(VERSION_PLACEHOLDER) {
            return Err(TagError);
        }
        Ok((start, end))
    }
}

#[derive(Debug)]
pub struct TagError;

impl fmt::Display for TagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tag.format must contain {version} exactly once")
    }
}

impl Error for TagError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analyzer {
    pub convention: AnalyzerConvention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalyzerConvention {
    Gitmoji,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubReleasePublisher {
    pub repository: String,
    pub token: Token,
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
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "unsupported declaration schemaVersion {version}")
            }
            Self::InvalidDeclaration(error) => write!(formatter, "invalid declaration: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl Error for DeclarationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) | Self::InvalidDeclaration(error) => Some(error),
            _ => None,
        }
    }
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

    const DECLARATION: &str = r#"{
        "schemaVersion": 1,
        "version": "1.2.3",
        "versionFiles": [
            { "path": "Cargo.toml", "toml": "package.version" },
            { "path": "Cargo.lock", "cargoLock": "presswerk" }
        ],
        "tag": { "format": "v{version}" },
        "analyzer": { "convention": "gitmoji" },
        "changelog": { "path": "CHANGELOG.md" },
        "publishers": {
            "github-release": {
                "default": {
                    "repository": "jkuball/presswerk",
                    "token": { "secret": "GITHUB_TOKEN" }
                }
            }
        }
    }"#;

    #[test]
    fn parses_the_m1_schema_with_a_required_changelog() {
        let declaration = ReleaseDeclaration::from_json(DECLARATION).unwrap();
        assert_eq!(declaration.schema_version, 1);
        assert_eq!(declaration.version, Version::new(1, 2, 3));
        assert_eq!(
            declaration.changelog,
            Changelog {
                path: "CHANGELOG.md".into()
            }
        );
    }

    #[test]
    fn rejects_a_missing_changelog() {
        let missing = DECLARATION.replace(
            "        \"changelog\": { \"path\": \"CHANGELOG.md\" },\n",
            "",
        );
        assert!(matches!(
            ReleaseDeclaration::from_json(&missing).unwrap_err(),
            DeclarationError::InvalidDeclaration(_)
        ));
    }

    #[test]
    fn rejects_an_unsupported_schema_before_the_remainder() {
        let error = ReleaseDeclaration::from_json(
            r#"{ "schemaVersion": 2, "version": false, "unexpected": true }"#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DeclarationError::UnsupportedSchemaVersion(2)
        ));
    }

    #[test]
    fn validates_repository_relative_paths() {
        let invalid = DECLARATION.replace("Cargo.toml", "../Cargo.toml");
        assert!(
            ReleaseDeclaration::from_json(&invalid)
                .unwrap_err()
                .to_string()
                .contains("repository-relative")
        );
    }

    #[test]
    fn validates_generic_toml_selectors() {
        let generic = DECLARATION.replace("package.version", "project.version");
        ReleaseDeclaration::from_json(&generic).unwrap();

        let empty = DECLARATION.replace("package.version", "");
        assert!(
            ReleaseDeclaration::from_json(&empty)
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn validates_the_tag_version_placeholder() {
        for format in ["v{version}", "release/{version}/source"] {
            let declaration = DECLARATION.replace("v{version}", format);
            ReleaseDeclaration::from_json(&declaration).unwrap();
        }

        for format in [
            "release",
            "{version}-{version}",
            "v{{ version }}",
            "v{% raw %}{{ version }}{% endraw %}",
        ] {
            let declaration = DECLARATION.replace("v{version}", format);
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
        for format in ["v{version}", "release/{version}/source"] {
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
    fn accepts_logical_secret_names_and_rejects_empty_names() {
        let logical_name = DECLARATION.replace("GITHUB_TOKEN", "github-token");
        ReleaseDeclaration::from_json(&logical_name).unwrap();

        let empty_name = DECLARATION.replace("GITHUB_TOKEN", "");
        assert!(
            ReleaseDeclaration::from_json(&empty_name)
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn rejects_invalid_semantic_versions() {
        let invalid = DECLARATION.replace("1.2.3", "next");
        assert!(matches!(
            ReleaseDeclaration::from_json(&invalid).unwrap_err(),
            DeclarationError::InvalidDeclaration(_)
        ));
    }
}
