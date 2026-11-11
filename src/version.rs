use crate::declaration::{
    BadgeVersionFile, CargoLockVersionFile, JsonVersionFile, PlainVersionFile, TomlVersionFile,
    VersionFile,
};
use jsonc_parser::ast::Value as JsonValue;
use jsonc_parser::{CollectOptions, ParseOptions, parse_to_ast};
use semver::Version;
use std::error::Error;
use std::fmt;
use toml_edit::{Document, Item, Table, Value};

pub trait VersionFileWriter {
    fn path(&self) -> &str;

    /// Whether the file carries the current version. A file that does not is only written, and
    /// `write_version` must not depend on its previous contents, which may be missing.
    fn reads_version(&self) -> bool {
        true
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError>;

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError>;
}

impl VersionFileWriter for VersionFile {
    fn path(&self) -> &str {
        match self {
            Self::Toml(file) => file.path(),
            Self::Json(file) => file.path(),
            Self::Plain(file) => file.path(),
            Self::CargoLock(file) => file.path(),
            Self::Badge(file) => file.path(),
        }
    }

    fn reads_version(&self) -> bool {
        match self {
            Self::Toml(file) => file.reads_version(),
            Self::Json(file) => file.reads_version(),
            Self::Plain(file) => file.reads_version(),
            Self::CargoLock(file) => file.reads_version(),
            Self::Badge(file) => file.reads_version(),
        }
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        match self {
            Self::Toml(file) => file.read_version(contents),
            Self::Json(file) => file.read_version(contents),
            Self::Plain(file) => file.read_version(contents),
            Self::CargoLock(file) => file.read_version(contents),
            Self::Badge(file) => file.read_version(contents),
        }
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        match self {
            Self::Toml(file) => file.write_version(contents, version),
            Self::Json(file) => file.write_version(contents, version),
            Self::Plain(file) => file.write_version(contents, version),
            Self::CargoLock(file) => file.write_version(contents, version),
            Self::Badge(file) => file.write_version(contents, version),
        }
    }
}

impl VersionFileWriter for TomlVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let item = find_item(document.as_item(), &self.toml).ok_or_else(|| {
            VersionFileError::Invalid(format!(
                "{} does not contain TOML key {}",
                self.path, self.toml
            ))
        })?;
        parse_version(
            &self.path,
            item.as_str().ok_or_else(|| {
                VersionFileError::Invalid(format!(
                    "TOML key {} in {} must be a string",
                    self.toml, self.path
                ))
            })?,
        )
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let item = find_item(document.as_item(), &self.toml).ok_or_else(|| {
            VersionFileError::Invalid(format!(
                "{} does not contain TOML key {}",
                self.path, self.toml
            ))
        })?;
        if !item.is_str() {
            return Err(VersionFileError::Invalid(format!(
                "TOML key {} in {} must be a string",
                self.toml, self.path
            )));
        }
        replace_string(&self.path, contents, item, version)
    }
}

impl JsonVersionFile {
    /// The byte range of the version string, including its quotes, and its value.
    fn locate(&self, contents: &str) -> Result<(std::ops::Range<usize>, String), VersionFileError> {
        let parsed = parse_to_ast(
            contents,
            &CollectOptions::default(),
            &ParseOptions::default(),
        )
        .map_err(|error| {
            VersionFileError::Invalid(format!("could not parse {}: {error}", self.path))
        })?;
        let mut value = parsed.value.as_ref();
        for key in self.json.split('.') {
            value = match value {
                Some(JsonValue::Object(object)) => object.get(key).map(|property| &property.value),
                _ => None,
            };
        }
        let missing = || {
            VersionFileError::Invalid(format!(
                "{} does not contain JSON key {}",
                self.path, self.json
            ))
        };
        match value.ok_or_else(missing)? {
            JsonValue::StringLit(string) => Ok((
                string.range.start..string.range.end,
                string.value.to_string(),
            )),
            _ => Err(VersionFileError::Invalid(format!(
                "JSON key {} in {} must be a string",
                self.json, self.path
            ))),
        }
    }
}

impl VersionFileWriter for JsonVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let (_, version) = self.locate(contents)?;
        parse_version(&self.path, &version)
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let (range, _) = self.locate(contents)?;
        let mut written = contents.to_owned();
        written.replace_range(range, &format!("\"{version}\""));
        Ok(written)
    }
}

impl VersionFileWriter for PlainVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let (version, _) = split_line_break(contents);
        parse_version(&self.path, version)
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let (_, line_break) = split_line_break(contents);
        Ok(format!("{version}{line_break}"))
    }
}

/// Split off one trailing line break, keeping whether it is LF or CRLF.
fn split_line_break(contents: &str) -> (&str, &str) {
    if let Some(version) = contents.strip_suffix("\r\n") {
        (version, "\r\n")
    } else if let Some(version) = contents.strip_suffix('\n') {
        (version, "\n")
    } else {
        (contents, "")
    }
}

impl VersionFileWriter for CargoLockVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let package = cargo_package(document.as_item(), &self.path, &self.cargo_lock)?;
        let version = package
            .get("version")
            .and_then(Item::as_str)
            .ok_or_else(|| {
                VersionFileError::Invalid(format!(
                    "package {} in {} has no string version",
                    self.cargo_lock, self.path
                ))
            })?;
        parse_version(&self.path, version)
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let package = cargo_package(document.as_item(), &self.path, &self.cargo_lock)?;
        let item = package.get("version").ok_or_else(|| {
            VersionFileError::Invalid(format!(
                "package {} in {} has no version",
                self.cargo_lock, self.path
            ))
        })?;
        if !item.is_str() {
            return Err(VersionFileError::Invalid(format!(
                "package {} in {} has no string version",
                self.cargo_lock, self.path
            )));
        }
        replace_string(&self.path, contents, item, version)
    }
}

impl VersionFileWriter for BadgeVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn reads_version(&self) -> bool {
        false
    }

    fn read_version(&self, _contents: &str) -> Result<Version, VersionFileError> {
        Err(VersionFileError::Invalid(format!(
            "{} is a badge and does not carry the version",
            self.path
        )))
    }

    fn write_version(
        &self,
        _contents: &str,
        version: &Version,
    ) -> Result<String, VersionFileError> {
        Ok(crate::badge::render(version, self.badge.color_scheme))
    }
}

pub fn check_consistent_versions(
    files: &[(&dyn VersionFileWriter, &str)],
) -> Result<Version, VersionFileError> {
    let mut sources = files.iter().filter(|(writer, _)| writer.reads_version());
    let Some((first_writer, first_contents)) = sources.next() else {
        return Err(VersionFileError::Invalid(
            "at least one version file that carries the version is required".into(),
        ));
    };
    let expected = first_writer.read_version(first_contents)?;

    for (writer, contents) in sources {
        let actual = writer.read_version(contents)?;
        if actual != expected {
            return Err(VersionFileError::Invalid(format!(
                "version files disagree: {} contains {expected}, but {} contains {actual}",
                first_writer.path(),
                writer.path()
            )));
        }
    }

    Ok(expected)
}

fn replace_string(
    path: &str,
    contents: &str,
    item: &Item,
    version: &Version,
) -> Result<String, VersionFileError> {
    let span = item
        .as_value()
        .and_then(Value::span)
        .ok_or_else(|| VersionFileError::Invalid(format!("could not locate version in {path}")))?;
    let replacement = Value::from(version.to_string()).to_string();
    let mut written = contents.to_owned();
    written.replace_range(span, &replacement);
    Ok(written)
}

fn parse_toml<'a>(path: &str, contents: &'a str) -> Result<Document<&'a str>, VersionFileError> {
    Document::parse(contents).map_err(|source| VersionFileError::Toml {
        path: path.into(),
        source,
    })
}

fn parse_version(path: &str, version: &str) -> Result<Version, VersionFileError> {
    Version::parse(version).map_err(|source| VersionFileError::Version {
        path: path.into(),
        version: version.into(),
        source,
    })
}

fn find_item<'a>(mut item: &'a Item, selector: &str) -> Option<&'a Item> {
    for key in selector.split('.') {
        item = item.as_table_like()?.get(key)?;
    }
    Some(item)
}

fn cargo_package<'a>(
    document: &'a Item,
    path: &str,
    name: &str,
) -> Result<&'a Table, VersionFileError> {
    let packages = document
        .as_table_like()
        .and_then(|document| document.get("package"))
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| VersionFileError::Invalid(format!("{path} has no package entries")))?;
    let mut matching = packages.iter().filter(|package| {
        package
            .get("name")
            .and_then(Item::as_str)
            .is_some_and(|package_name| package_name == name)
    });
    let package = matching
        .next()
        .ok_or_else(|| VersionFileError::Invalid(format!("{path} has no package named {name}")))?;
    if matching.next().is_some() {
        return Err(VersionFileError::Invalid(format!(
            "{path} has more than one package named {name}"
        )));
    }
    Ok(package)
}

#[derive(Debug)]
pub enum VersionFileError {
    Invalid(String),
    Toml {
        path: String,
        source: toml_edit::TomlError,
    },
    Version {
        path: String,
        version: String,
        source: semver::Error,
    },
}

impl fmt::Display for VersionFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Toml { path, source } => write!(formatter, "could not parse {path}: {source}"),
            Self::Version {
                path,
                version,
                source,
            } => write!(formatter, "invalid version {version:?} in {path}: {source}"),
        }
    }
}

impl Error for VersionFileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Toml { source, .. } => Some(source),
            Self::Version { source, .. } => Some(source),
            Self::Invalid(_) => None,
        }
    }
}
