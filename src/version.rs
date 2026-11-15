use crate::declaration::{
    BadgeVersionFile, CargoLockVersionFile, JsonVersionFile, PlainVersionFile, RegexVersionFile,
    TomlVersionFile, VersionFile, key_segments,
};
use glob::{MatchOptions, Pattern};
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

/// The files of the commit that version files are read from, for writers that look beyond their
/// own file.
pub trait SourceTree {
    /// The contents of a repository-relative file, or `None` if the commit does not contain it.
    fn read(&self, path: &str) -> Result<Option<String>, VersionFileError>;

    /// Every directory of the commit as a repository-relative path without a trailing slash.
    fn directories(&self) -> Result<Vec<String>, VersionFileError>;
}

impl VersionFile {
    /// The writer for this file. A Cargo workspace first resolves its members in `tree`.
    pub fn writer(
        &self,
        tree: &dyn SourceTree,
    ) -> Result<Box<dyn VersionFileWriter + '_>, VersionFileError> {
        Ok(match self {
            Self::Toml(file) => Box::new(file),
            Self::Json(file) => Box::new(file),
            Self::Plain(file) => Box::new(file),
            Self::Regex(file) => Box::new(file),
            Self::CargoLock(file) => Box::new(file),
            Self::CargoWorkspace(file) => Box::new(CargoWorkspaceLock {
                path: file.path.clone(),
                packages: cargo_workspace_packages(&file.cargo_workspace, tree)?,
            }),
            Self::Badge(file) => Box::new(file),
        })
    }
}

impl<T: VersionFileWriter + ?Sized> VersionFileWriter for &T {
    fn path(&self) -> &str {
        (**self).path()
    }

    fn reads_version(&self) -> bool {
        (**self).reads_version()
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        (**self).read_version(contents)
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        (**self).write_version(contents, version)
    }
}

impl VersionFileWriter for TomlVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let item = find_item(
            document.as_item(),
            segments(&self.path, "TOML", &self.toml)?,
        )
        .ok_or_else(|| {
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
        let item = find_item(
            document.as_item(),
            segments(&self.path, "TOML", &self.toml)?,
        )
        .ok_or_else(|| {
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
        for key in segments(&self.path, "JSON", &self.json)? {
            value = match value {
                Some(JsonValue::Object(object)) => object.get(&key).map(|property| &property.value),
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

impl RegexVersionFile {
    /// The byte range of the `version` group in the pattern's only match.
    fn locate(&self, contents: &str) -> Result<std::ops::Range<usize>, VersionFileError> {
        let pattern = self
            .compile()
            .map_err(|error| VersionFileError::Invalid(error.to_string()))?;
        let mut matches = pattern.captures_iter(contents);
        let found = matches.next().ok_or_else(|| {
            VersionFileError::Invalid(format!("pattern for {} does not match", self.path))
        })?;
        if matches.next().is_some() {
            return Err(VersionFileError::Invalid(format!(
                "pattern for {} matches more than once",
                self.path
            )));
        }
        found
            .name("version")
            .map(|group| group.range())
            .ok_or_else(|| {
                VersionFileError::Invalid(format!(
                    "pattern for {} matches without its version group",
                    self.path
                ))
            })
    }
}

impl VersionFileWriter for RegexVersionFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let range = self.locate(contents)?;
        parse_version(&self.path, &contents[range])
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let range = self.locate(contents)?;
        let mut written = contents.to_owned();
        written.replace_range(range, &version.to_string());
        Ok(written)
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

/// The package entries in `Cargo.lock` of the workspace members that inherit the workspace
/// version. They must all carry the same version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoWorkspaceLock {
    pub path: String,
    pub packages: Vec<String>,
}

impl CargoWorkspaceLock {
    /// The version items of all packages, each with its package name.
    fn versions<'a>(&self, document: &'a Item) -> Result<Vec<(&str, &'a Item)>, VersionFileError> {
        self.packages
            .iter()
            .map(|name| {
                let package = cargo_package(document, &self.path, name)?;
                let item = package
                    .get("version")
                    .filter(|item| item.is_str())
                    .ok_or_else(|| {
                        VersionFileError::Invalid(format!(
                            "package {name} in {} has no string version",
                            self.path
                        ))
                    })?;
                Ok((name.as_str(), item))
            })
            .collect()
    }
}

impl VersionFileWriter for CargoWorkspaceLock {
    fn path(&self) -> &str {
        &self.path
    }

    fn read_version(&self, contents: &str) -> Result<Version, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let mut expected: Option<(&str, Version)> = None;
        for (name, item) in self.versions(document.as_item())? {
            let version = parse_version(&self.path, item.as_str().unwrap_or_default())?;
            match &expected {
                None => expected = Some((name, version)),
                Some((first, first_version)) if *first_version != version => {
                    return Err(VersionFileError::Invalid(format!(
                        "workspace members disagree in {}: {first} has {first_version}, but {name} has {version}",
                        self.path
                    )));
                }
                Some(_) => {}
            }
        }
        expected.map(|(_, version)| version).ok_or_else(|| {
            VersionFileError::Invalid(format!("{} has no workspace members to read", self.path))
        })
    }

    fn write_version(&self, contents: &str, version: &Version) -> Result<String, VersionFileError> {
        let document = parse_toml(&self.path, contents)?;
        let mut spans = self
            .versions(document.as_item())?
            .into_iter()
            .map(|(_, item)| {
                item.as_value().and_then(Value::span).ok_or_else(|| {
                    VersionFileError::Invalid(format!("could not locate version in {}", self.path))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Replace from the end so that earlier spans stay valid.
        spans.sort_by_key(|span| std::cmp::Reverse(span.start));
        let replacement = Value::from(version.to_string()).to_string();
        let mut written = contents.to_owned();
        for span in spans {
            written.replace_range(span, &replacement);
        }
        Ok(written)
    }
}

/// The names of the members of the workspace in `manifest` that inherit the workspace version,
/// including the root package. Members come from `workspace.members`, whose globs match
/// directories, minus the directories under a `workspace.exclude` entry.
pub fn cargo_workspace_packages(
    manifest: &str,
    tree: &dyn SourceTree,
) -> Result<Vec<String>, VersionFileError> {
    let contents = tree.read(manifest)?.ok_or_else(|| {
        VersionFileError::Invalid(format!("commit does not contain required file {manifest}"))
    })?;
    let document = parse_toml(manifest, &contents)?;
    let workspace = document
        .get("workspace")
        .and_then(Item::as_table_like)
        .ok_or_else(|| VersionFileError::Invalid(format!("{manifest} has no [workspace] table")))?;
    let root = manifest.rsplit_once('/').map_or("", |(root, _)| root);
    let paths = |key: &str| -> Result<Vec<String>, VersionFileError> {
        let Some(item) = workspace.get(key) else {
            return Ok(Vec::new());
        };
        let invalid = || {
            VersionFileError::Invalid(format!(
                "workspace.{key} in {manifest} must be a list of strings"
            ))
        };
        item.as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|entry| {
                let entry = entry.as_str().ok_or_else(invalid)?;
                join_path(root, entry).ok_or_else(|| {
                    VersionFileError::Invalid(format!(
                        "workspace.{key} entry {entry:?} in {manifest} leaves the repository"
                    ))
                })
            })
            .collect()
    };
    let excluded = paths("exclude")?;

    let mut directories = Vec::new();
    let mut tree_directories = None;
    for member in paths("members")? {
        if !member.contains(['*', '?', '[']) {
            directories.push(member);
            continue;
        }
        let pattern = Pattern::new(&member).map_err(|error| {
            VersionFileError::Invalid(format!(
                "invalid workspace member pattern {member:?} in {manifest}: {error}"
            ))
        })?;
        let options = MatchOptions {
            require_literal_separator: true,
            ..MatchOptions::new()
        };
        let all = match &tree_directories {
            Some(all) => all,
            None => tree_directories.insert(tree.directories()?),
        };
        let mut matched: Vec<String> = all
            .iter()
            .filter(|directory| pattern.matches_with(directory, options))
            .cloned()
            .collect();
        matched.sort();
        directories.extend(matched);
    }
    directories.retain(|directory| {
        !excluded
            .iter()
            .any(|excluded| directory == excluded || directory.starts_with(&format!("{excluded}/")))
    });

    let mut packages = Vec::new();
    if let Some(name) = inherited_package(manifest, &document)? {
        packages.push(name);
    }
    for directory in directories {
        let path = if directory.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{directory}/Cargo.toml")
        };
        if path == manifest {
            continue;
        }
        let contents = tree.read(&path)?.ok_or_else(|| {
            VersionFileError::Invalid(format!(
                "workspace member {directory} of {manifest} has no Cargo.toml"
            ))
        })?;
        if let Some(name) = inherited_package(&path, &parse_toml(&path, &contents)?)?
            && !packages.contains(&name)
        {
            packages.push(name);
        }
    }
    if packages.is_empty() {
        return Err(VersionFileError::Invalid(format!(
            "no package of the workspace in {manifest} inherits workspace.package.version"
        )));
    }
    Ok(packages)
}

/// The package name of a manifest whose version is `version.workspace = true`.
fn inherited_package(
    path: &str,
    document: &Document<&str>,
) -> Result<Option<String>, VersionFileError> {
    let Some(package) = document.get("package").and_then(Item::as_table_like) else {
        return Ok(None);
    };
    let inherits = package
        .get("version")
        .and_then(Item::as_table_like)
        .and_then(|version| version.get("workspace"))
        .and_then(Item::as_bool)
        == Some(true);
    if !inherits {
        return Ok(None);
    }
    package
        .get("name")
        .and_then(Item::as_str)
        .map(|name| Some(name.to_owned()))
        .ok_or_else(|| VersionFileError::Invalid(format!("{path} has no string package.name")))
}

/// `path` relative to the repository-relative directory `root`, normalized, or `None` if it leaves
/// the repository.
fn join_path(root: &str, path: &str) -> Option<String> {
    let mut components: Vec<&str> = root.split('/').filter(|part| !part.is_empty()).collect();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component),
        }
    }
    Some(components.join("/"))
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

fn segments(path: &str, format: &str, key: &str) -> Result<Vec<String>, VersionFileError> {
    key_segments(key)
        .map_err(|error| VersionFileError::Invalid(format!("{format} key for {path} {error}")))
}

fn find_item(mut item: &Item, segments: Vec<String>) -> Option<&Item> {
    for key in &segments {
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
