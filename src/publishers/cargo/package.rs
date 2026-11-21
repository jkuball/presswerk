//! The local `.crate` archive: its bytes, checksum, and the metadata the publish request needs.
//!
//! The archive is read only as far as the publish API needs it: the packaged `Cargo.toml` (and the
//! readme it names) becomes the request's metadata. Its content is never validated; the registry
//! rejects what it does not accept.

use crate::publication::PublicationError;
use crate::publishers::shared::digest::sha256_digest;
use flate2::read::GzDecoder;
use semver::{Version, VersionReq};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The index URL that a dependency without `registry-index` comes from.
pub const CRATES_IO_INDEX: &str = "https://github.com/rust-lang/crates.io-index";
/// The sparse index of crates.io, which a dependency from crates.io names as its own registry.
const CRATES_IO_SPARSE_INDEX: &str = "https://index.crates.io";

/// Upper bound for a file read from the archive, so a small archive cannot expand without limit.
const ENTRY_LIMIT: u64 = 16 << 20;

/// The `.crate` archive of one release.
#[derive(Debug)]
pub struct LocalCrate {
    /// The archive as declared, a store path.
    pub origin: String,
    pub bytes: Vec<u8>,
    /// The hex SHA-256 of the archive, as the index records it in `cksum`.
    pub cksum: String,
    pub name: String,
    pub version: Version,
    /// The metadata of the publish request.
    pub metadata: Value,
}

fn crate_error(path: &Path, action: &str, error: impl std::fmt::Display) -> PublicationError {
    PublicationError::new(format!(
        "could not {action} crate archive {}: {error}",
        path.display()
    ))
}

/// The archive in `path`: the file itself, or the single `*.crate` file of a directory, such as
/// the output of a derivation that copies `target/package/*.crate`.
pub fn archive_path(path: &Path) -> Result<PathBuf, PublicationError> {
    if !path.is_dir() {
        return Ok(path.to_owned());
    }
    let mut archives = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| crate_error(path, "list", error))? {
        let entry = entry
            .map_err(|error| crate_error(path, "list", error))?
            .path();
        if entry
            .extension()
            .is_some_and(|extension| extension == "crate")
            && entry.is_file()
        {
            archives.push(entry);
        }
    }
    match <[PathBuf; 1]>::try_from(archives) {
        Ok([archive]) => Ok(archive),
        Err(archives) => Err(PublicationError::new(format!(
            "{} must be a .crate file or a directory with exactly one, found {} .crate files",
            path.display(),
            archives.len()
        ))),
    }
}

impl LocalCrate {
    /// Reads the archive at `origin` for `release_version` and the registry with sparse index
    /// `index` (without the `sparse+` prefix).
    pub fn open(
        origin: &str,
        index: &str,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        let path = archive_path(Path::new(origin))?;
        let bytes = fs::read(&path).map_err(|error| crate_error(&path, "read", error))?;
        Self::from_bytes(origin, &path, bytes, index, release_version)
    }

    fn from_bytes(
        origin: &str,
        path: &Path,
        bytes: Vec<u8>,
        index: &str,
        release_version: &Version,
    ) -> Result<Self, PublicationError> {
        let (root, manifest) = read_entry(&bytes, path, |_, name| name == "Cargo.toml")?
            .ok_or_else(|| {
                PublicationError::new(format!(
                    "crate archive {} has no top-level Cargo.toml",
                    path.display()
                ))
            })?;
        let manifest = std::str::from_utf8(&manifest)
            .map_err(|error| crate_error(path, "read Cargo.toml in", error))?;
        let manifest = toml_to_json(
            manifest
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| crate_error(path, "parse Cargo.toml in", error))?
                .as_item(),
        );
        let package = manifest.get("package").unwrap_or(&Value::Null);
        let field = |name: &str| {
            package
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PublicationError::new(format!(
                        "Cargo.toml in {} has no package.{name}",
                        path.display()
                    ))
                })
                .map(str::to_owned)
        };
        let name = field("name")?;
        let version_text = field("version")?;
        let version = Version::parse(&version_text).map_err(|error| {
            PublicationError::new(format!(
                "crate {name:?} in {} has invalid version {version_text:?}: {error}",
                path.display()
            ))
        })?;
        if version != *release_version {
            return Err(PublicationError::new(format!(
                "crate archive {} has version {version}, expected release version {release_version}",
                path.display()
            )));
        }
        let readme = match package.get("readme").and_then(Value::as_str) {
            Some(readme) => Some(read_readme(&bytes, path, &root, readme)?),
            None => None,
        };
        let metadata = metadata(&manifest, readme, index);
        Ok(Self {
            origin: origin.to_owned(),
            cksum: sha256_digest(&bytes)
                .trim_start_matches("sha256:")
                .to_owned(),
            bytes,
            name,
            version,
            metadata,
        })
    }

    /// The body of `PUT /api/v1/crates/new`: the metadata JSON and the archive, each preceded by
    /// its length as a little-endian `u32`.
    #[must_use]
    pub fn publish_body(&self) -> Vec<u8> {
        let metadata = serde_json::to_vec(&self.metadata).expect("crate metadata serializes");
        let mut body = Vec::with_capacity(8 + metadata.len() + self.bytes.len());
        for part in [&metadata, &self.bytes] {
            let length = u32::try_from(part.len()).expect("crate parts are smaller than 4 GiB");
            body.extend_from_slice(&length.to_le_bytes());
            body.extend_from_slice(part);
        }
        body
    }
}

/// The readme at `readme`, relative to the package root `root` in the archive.
fn read_readme(
    bytes: &[u8],
    path: &Path,
    root: &str,
    readme: &str,
) -> Result<String, PublicationError> {
    let wanted: Vec<_> = Path::new(root)
        .join(readme)
        .components()
        .filter(|component| *component != Component::CurDir)
        .map(|component| component.as_os_str().to_owned())
        .collect();
    let readme_bytes = read_matching(bytes, path, &wanted)?.ok_or_else(|| {
        PublicationError::new(format!(
            "crate archive {} has no readme {readme:?}",
            path.display()
        ))
    })?;
    Ok(String::from_utf8_lossy(&readme_bytes).into_owned())
}

fn read_matching(
    bytes: &[u8],
    path: &Path,
    wanted: &[std::ffi::OsString],
) -> Result<Option<Vec<u8>>, PublicationError> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    for entry in archive
        .entries()
        .map_err(|error| crate_error(path, "read", error))?
    {
        let entry = entry.map_err(|error| crate_error(path, "read", error))?;
        let entry_path = entry
            .path()
            .map_err(|error| crate_error(path, "read a path in", error))?;
        let components: Vec<_> = entry_path
            .components()
            .map(|component| component.as_os_str().to_owned())
            .collect();
        if components == wanted {
            return read_limited(entry, path).map(Some);
        }
    }
    Ok(None)
}

/// The first entry `<root>/<name>` for which `matches(root, name)` holds, with its root.
fn read_entry(
    bytes: &[u8],
    path: &Path,
    matches: impl Fn(&str, &str) -> bool,
) -> Result<Option<(String, Vec<u8>)>, PublicationError> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    for entry in archive
        .entries()
        .map_err(|error| crate_error(path, "read", error))?
    {
        let entry = entry.map_err(|error| crate_error(path, "read", error))?;
        let entry_path = entry
            .path()
            .map_err(|error| crate_error(path, "read a path in", error))?;
        let components: Vec<_> = entry_path.components().collect();
        let [Component::Normal(root), Component::Normal(name)] = components.as_slice() else {
            continue;
        };
        let (Some(root), Some(name)) = (root.to_str(), name.to_str()) else {
            continue;
        };
        if matches(root, name) {
            let root = root.to_owned();
            return Ok(Some((root, read_limited(entry, path)?)));
        }
    }
    Ok(None)
}

fn read_limited(entry: impl Read, path: &Path) -> Result<Vec<u8>, PublicationError> {
    let mut bytes = Vec::new();
    entry
        .take(ENTRY_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| crate_error(path, "read", error))?;
    if bytes.len() as u64 > ENTRY_LIMIT {
        return Err(PublicationError::new(format!(
            "a file in crate archive {} is larger than 16 MiB",
            path.display()
        )));
    }
    Ok(bytes)
}

fn toml_to_json(item: &toml_edit::Item) -> Value {
    match item {
        toml_edit::Item::None => Value::Null,
        toml_edit::Item::Value(value) => toml_value_to_json(value),
        toml_edit::Item::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.to_owned(), toml_to_json(item)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(tables) => Value::Array(
            tables
                .iter()
                .map(|table| toml_to_json(&toml_edit::Item::Table(table.clone())))
                .collect(),
        ),
    }
}

fn toml_value_to_json(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(value) => Value::String(value.value().clone()),
        toml_edit::Value::Integer(value) => json!(value.value()),
        toml_edit::Value::Float(value) => json!(value.value()),
        toml_edit::Value::Boolean(value) => Value::Bool(*value.value()),
        toml_edit::Value::Datetime(value) => Value::String(value.value().to_string()),
        toml_edit::Value::Array(array) => {
            Value::Array(array.iter().map(toml_value_to_json).collect())
        }
        toml_edit::Value::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), toml_value_to_json(value)))
                .collect(),
        ),
    }
}

/// An index URL without `sparse+` and trailing slashes, so two spellings of one registry compare
/// equal.
fn registry_identity(index: &str) -> &str {
    index.trim_start_matches("sparse+").trim_end_matches('/')
}

/// The metadata of the publish request, as Cargo sends it, from the packaged manifest.
fn metadata(manifest: &Value, readme: Option<String>, index: &str) -> Value {
    let package = manifest.get("package").unwrap_or(&Value::Null);
    let text = |name: &str| package.get(name).and_then(Value::as_str).map(str::to_owned);
    let list = |name: &str| package.get(name).cloned().unwrap_or_else(|| json!([]));
    json!({
        "name": text("name"),
        "vers": text("version"),
        "deps": dependencies(manifest, index),
        "features": manifest.get("features").cloned().unwrap_or_else(|| json!({})),
        "authors": list("authors"),
        "description": text("description"),
        "documentation": text("documentation"),
        "homepage": text("homepage"),
        "readme": readme,
        "readme_file": text("readme"),
        "keywords": list("keywords"),
        "categories": list("categories"),
        "license": text("license"),
        "license_file": text("license-file"),
        "repository": text("repository"),
        "badges": {},
        "links": text("links"),
        "rust_version": text("rust-version"),
    })
}

/// Every dependency of the manifest, of every kind and target, in the publish request's shape.
fn dependencies(manifest: &Value, index: &str) -> Vec<Value> {
    let target_is_crates_io = registry_identity(index) == CRATES_IO_SPARSE_INDEX;
    let mut tables = vec![(None, manifest)];
    if let Some(targets) = manifest.get("target").and_then(Value::as_object) {
        tables.extend(
            targets
                .iter()
                .map(|(target, table)| (Some(target.as_str()), table)),
        );
    }
    let mut deps = Vec::new();
    for (target, table) in tables {
        for (kind, keys) in [
            ("normal", &["dependencies"][..]),
            ("dev", &["dev-dependencies", "dev_dependencies"]),
            ("build", &["build-dependencies", "build_dependencies"]),
        ] {
            for key in keys {
                let Some(entries) = table.get(key).and_then(Value::as_object) else {
                    continue;
                };
                for (name, spec) in entries {
                    deps.push(dependency(
                        name,
                        spec,
                        kind,
                        target,
                        index,
                        target_is_crates_io,
                    ));
                }
            }
        }
    }
    deps
}

fn dependency(
    name: &str,
    spec: &Value,
    kind: &str,
    target: Option<&str>,
    index: &str,
    target_is_crates_io: bool,
) -> Value {
    let empty = Map::new();
    let (version, table) = match spec {
        Value::String(version) => (Some(version.as_str()), &empty),
        Value::Object(table) => (table.get("version").and_then(Value::as_str), table),
        _ => (None, &empty),
    };
    // Cargo sends the requirement as it parses it, such as `^1.0` for `1.0`.
    let version_req = version.map_or_else(
        || "*".to_owned(),
        |version| {
            VersionReq::parse(version).map_or_else(|_| version.to_owned(), |req| req.to_string())
        },
    );
    let flag = |keys: &[&str], default: bool| {
        keys.iter()
            .find_map(|key| table.get(*key).and_then(Value::as_bool))
            .unwrap_or(default)
    };
    let package = table.get("package").and_then(Value::as_str);
    let registry = match table.get("registry-index").and_then(Value::as_str) {
        Some(dependency_index)
            if registry_identity(dependency_index) == registry_identity(index) =>
        {
            None
        }
        Some(dependency_index) => Some(dependency_index.to_owned()),
        None if target_is_crates_io => None,
        None => Some(CRATES_IO_INDEX.to_owned()),
    };
    json!({
        "name": package.unwrap_or(name),
        "version_req": version_req,
        "features": table.get("features").cloned().unwrap_or_else(|| json!([])),
        "optional": flag(&["optional"], false),
        "default_features": flag(&["default-features", "default_features"], true),
        "target": target,
        "kind": kind,
        "registry": registry,
        "explicit_name_in_toml": package.map(|_| name),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    /// A `.crate` archive with `files` below `<name>-<version>/`.
    pub(crate) fn archive(name: &str, version: &str, files: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (file, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(1);
            builder
                .append_data(
                    &mut header,
                    format!("{name}-{version}/{file}"),
                    contents.as_bytes(),
                )
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    pub(crate) fn manifest(name: &str, version: &str) -> String {
        format!(
            r#"[package]
edition = "2021"
name = "{name}"
version = "{version}"
authors = ["Jane Doe"]
description = "A test crate"
readme = "README.md"
license = "MIT"
keywords = ["test"]

[features]
default = ["std"]
std = []

[dependencies.serde]
version = "1"
features = ["derive"]

[dependencies.sibling]
version = "=1.2.3"
registry-index = "sparse+https://kellnr.example.com/api/v1/crates/"

[dependencies.renamed]
version = "0.4"
package = "original"
optional = true
default-features = false

[dev-dependencies.tempfile]
version = "3"

[target."cfg(unix)".build-dependencies]
cc = "1.0"
"#
        )
    }

    fn open(bytes: Vec<u8>, index: &str, version: &str) -> Result<LocalCrate, PublicationError> {
        LocalCrate::from_bytes(
            "/nix/store/abc-demo",
            Path::new("demo.crate"),
            bytes,
            index,
            &Version::parse(version).unwrap(),
        )
    }

    #[test]
    fn reads_the_publish_metadata_from_the_packaged_manifest() {
        let manifest = manifest("demo", "1.2.3");
        let bytes = archive(
            "demo",
            "1.2.3",
            &[
                ("Cargo.toml.orig", "ignored"),
                ("Cargo.toml", &manifest),
                ("README.md", "# Demo\n"),
                ("src/lib.rs", ""),
            ],
        );
        let local = open(bytes.clone(), "sparse+https://index.crates.io/", "1.2.3").unwrap();

        assert_eq!(local.name, "demo");
        assert_eq!(
            local.cksum,
            sha256_digest(&bytes).trim_start_matches("sha256:")
        );
        let metadata = &local.metadata;
        assert_eq!(metadata["name"], "demo");
        assert_eq!(metadata["vers"], "1.2.3");
        assert_eq!(metadata["readme"], "# Demo\n");
        assert_eq!(metadata["readme_file"], "README.md");
        assert_eq!(metadata["authors"], json!(["Jane Doe"]));
        assert_eq!(
            metadata["features"],
            json!({ "default": ["std"], "std": [] })
        );
        assert_eq!(metadata["homepage"], Value::Null);
        assert_eq!(
            metadata["deps"],
            json!([
                {
                    "name": "original", "version_req": "^0.4", "features": [],
                    "optional": true, "default_features": false, "target": null,
                    "kind": "normal", "registry": null, "explicit_name_in_toml": "renamed"
                },
                {
                    "name": "serde", "version_req": "^1", "features": ["derive"],
                    "optional": false, "default_features": true, "target": null,
                    "kind": "normal", "registry": null, "explicit_name_in_toml": null
                },
                {
                    "name": "sibling", "version_req": "=1.2.3", "features": [],
                    "optional": false, "default_features": true, "target": null,
                    "kind": "normal",
                    "registry": "sparse+https://kellnr.example.com/api/v1/crates/",
                    "explicit_name_in_toml": null
                },
                {
                    "name": "tempfile", "version_req": "^3", "features": [],
                    "optional": false, "default_features": true, "target": null,
                    "kind": "dev", "registry": null, "explicit_name_in_toml": null
                },
                {
                    "name": "cc", "version_req": "^1.0", "features": [],
                    "optional": false, "default_features": true, "target": "cfg(unix)",
                    "kind": "build", "registry": null, "explicit_name_in_toml": null
                },
            ])
        );
    }

    #[test]
    fn dependencies_name_their_registry_unless_it_is_the_target() {
        let manifest = manifest("demo", "1.2.3");
        let bytes = archive(
            "demo",
            "1.2.3",
            &[("Cargo.toml", &manifest), ("README.md", "")],
        );
        let local = open(
            bytes,
            "sparse+https://kellnr.example.com/api/v1/crates",
            "1.2.3",
        )
        .unwrap();
        let registries: Vec<_> = local.metadata["deps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|dep| (dep["name"].as_str().unwrap(), dep["registry"].clone()))
            .collect();
        assert!(registries.contains(&("serde", json!(CRATES_IO_INDEX))));
        assert!(registries.contains(&("sibling", Value::Null)));
    }

    #[test]
    fn the_version_must_be_the_release_version_and_nothing_else_is_checked() {
        // No description, license, or readme: the registry decides what it accepts.
        let bytes = archive(
            "demo",
            "1.2.3",
            &[(
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\npublish = false\n",
            )],
        );
        let local = open(bytes.clone(), "sparse+https://index.crates.io/", "1.2.3").unwrap();
        assert_eq!(local.metadata["readme"], Value::Null);
        assert_eq!(local.metadata["deps"], json!([]));

        let error = open(bytes, "sparse+https://index.crates.io/", "1.2.4")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("has version 1.2.3, expected release version 1.2.4"),
            "{error}"
        );
    }

    #[test]
    fn the_body_frames_metadata_and_archive_with_their_lengths() {
        let bytes = archive(
            "demo",
            "1.2.3",
            &[(
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
            )],
        );
        let local = open(bytes.clone(), "sparse+https://index.crates.io/", "1.2.3").unwrap();
        let body = local.publish_body();
        let metadata_length = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
        let metadata: Value = serde_json::from_slice(&body[4..4 + metadata_length]).unwrap();
        assert_eq!(metadata["name"], "demo");
        let rest = &body[4 + metadata_length..];
        assert_eq!(
            u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize,
            bytes.len()
        );
        assert_eq!(&rest[4..], bytes.as_slice());
    }

    #[test]
    fn a_directory_holds_exactly_one_crate() {
        let directory = tempfile::tempdir().unwrap();
        let error = archive_path(directory.path()).unwrap_err().to_string();
        assert!(error.contains("found 0 .crate files"), "{error}");
        fs::write(directory.path().join("demo-1.2.3.crate"), b"").unwrap();
        fs::write(directory.path().join("notes.txt"), b"").unwrap();
        assert_eq!(
            archive_path(directory.path()).unwrap(),
            directory.path().join("demo-1.2.3.crate")
        );
        fs::write(directory.path().join("other-1.2.3.crate"), b"").unwrap();
        assert!(archive_path(directory.path()).is_err());
    }
}
