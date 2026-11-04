use crate::declaration::{CargoLockVersionFile, TomlVersionFile, VersionFile};
use semver::Version;
use std::error::Error;
use std::fmt;
use toml_edit::{DocumentMut, Item, Table, Value};

pub trait VersionFileWriter {
    fn path(&self) -> &str;

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError>;

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError>;
}

impl VersionFileWriter for VersionFile {
    fn path(&self) -> &str {
        match self {
            Self::Toml(file) => file.path(),
            Self::CargoLock(file) => file.path(),
        }
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        match self {
            Self::Toml(file) => file.read_version(contents),
            Self::CargoLock(file) => file.read_version(contents),
        }
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        match self {
            Self::Toml(file) => file.write_version(contents, version),
            Self::CargoLock(file) => file.write_version(contents, version),
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
        let mut document = parse_toml(&self.path, contents)?;
        let item = find_item_mut(document.as_item_mut(), &self.toml).ok_or_else(|| {
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
        replace_string(item, version.to_string());
        Ok(document.to_string())
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
        let mut document = parse_toml(&self.path, contents)?;
        let package = cargo_package_mut(document.as_item_mut(), &self.path, &self.cargo_lock)?;
        let item = package.get_mut("version").ok_or_else(|| {
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
        replace_string(item, version.to_string());
        Ok(document.to_string())
    }
}

pub fn check_consistent_versions(
    files: &[(&dyn VersionFileWriter, &str)],
) -> Result<Version, VersionFileError> {
    let Some((first_writer, first_contents)) = files.first() else {
        return Err(VersionFileError::Invalid(
            "at least one version file is required".into(),
        ));
    };
    let expected = first_writer.read_version(first_contents)?;

    for (writer, contents) in &files[1..] {
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

fn replace_string(item: &mut Item, contents: String) {
    let decor = item
        .as_value()
        .expect("string item is a value")
        .decor()
        .clone();
    let mut value = Value::from(contents);
    *value.decor_mut() = decor;
    *item = Item::Value(value);
}

fn parse_toml(path: &str, contents: &str) -> Result<DocumentMut, VersionFileError> {
    contents.parse().map_err(|source| VersionFileError::Toml {
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

fn find_item_mut<'a>(mut item: &'a mut Item, selector: &str) -> Option<&'a mut Item> {
    for key in selector.split('.') {
        item = item.as_table_like_mut()?.get_mut(key)?;
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

fn cargo_package_mut<'a>(
    document: &'a mut Item,
    path: &str,
    name: &str,
) -> Result<&'a mut Table, VersionFileError> {
    let packages = document
        .as_table_like_mut()
        .and_then(|document| document.get_mut("package"))
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| VersionFileError::Invalid(format!("{path} has no package entries")))?;
    let mut found = None;
    for (index, package) in packages.iter().enumerate() {
        if package
            .get("name")
            .and_then(Item::as_str)
            .is_some_and(|package_name| package_name == name)
            && found.replace(index).is_some()
        {
            return Err(VersionFileError::Invalid(format!(
                "{path} has more than one package named {name}"
            )));
        }
    }
    let index = found
        .ok_or_else(|| VersionFileError::Invalid(format!("{path} has no package named {name}")))?;
    Ok(packages.get_mut(index).expect("matching package exists"))
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
