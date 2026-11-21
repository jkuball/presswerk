//! The local distributions of a release: wheels and an sdist, with what the upload form needs.
//!
//! Presswerk reads each file's name, its core metadata, and its digest, and compares the version
//! with the release version. It never checks whether the metadata is valid: the index decides.

use crate::publication::PublicationError;
use crate::publishers::shared::digest::sha256_digest_reader;
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// What kind of distribution a file is, as the upload form's `filetype` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filetype {
    Wheel,
    Sdist,
}

impl Filetype {
    #[must_use]
    pub fn form_value(self) -> &'static str {
        match self {
            Self::Wheel => "bdist_wheel",
            Self::Sdist => "sdist",
        }
    }
}

/// What a distribution's filename says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filename {
    /// The project name as the filename spells it, such as `my_package`.
    pub name: String,
    pub version: String,
    pub filetype: Filetype,
    /// The upload form's `pyversion`: a wheel's Python tag, such as `py3`, or `source`.
    pub pyversion: String,
}

impl Filename {
    /// Reads a wheel (`{name}-{version}(-{build})?-{python}-{abi}-{platform}.whl`) or an sdist
    /// (`{name}-{version}.tar.gz`) filename; anything else is no distribution.
    #[must_use]
    pub fn parse(filename: &str) -> Option<Self> {
        if let Some(stem) = filename.strip_suffix(".whl") {
            let parts: Vec<&str> = stem.split('-').collect();
            if !matches!(parts.len(), 5 | 6) || parts.iter().any(|part| part.is_empty()) {
                return None;
            }
            return Some(Self {
                name: parts[0].to_owned(),
                version: parts[1].to_owned(),
                filetype: Filetype::Wheel,
                pyversion: parts[parts.len() - 3].to_owned(),
            });
        }
        let stem = filename.strip_suffix(".tar.gz")?;
        let (name, version) = stem.rsplit_once('-')?;
        if name.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            name: name.to_owned(),
            version: version.to_owned(),
            filetype: Filetype::Sdist,
            pyversion: "source".to_owned(),
        })
    }
}

/// A project name in its normalized form ([PEP 503]), such as `my-package` for `My_Package`.
///
/// [PEP 503]: https://peps.python.org/pep-0503/#normalized-names
#[must_use]
pub fn normalize(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut separator = false;
    for character in name.chars() {
        if matches!(character, '-' | '_' | '.') {
            separator = true;
            continue;
        }
        if separator && !normalized.is_empty() {
            normalized.push('-');
        }
        separator = false;
        normalized.push(character.to_ascii_lowercase());
    }
    normalized
}

/// One file to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distribution {
    pub filename: String,
    pub path: PathBuf,
    pub parsed: Filename,
    /// The SHA-256 digest of the file, as lowercase hex.
    pub sha256: String,
    /// The core metadata as upload form fields, in file order, such as `("name", "my-package")`.
    pub metadata: Vec<(String, String)>,
}

impl Distribution {
    /// Reads the distribution at `path`: its filename, digest, and core metadata.
    pub fn read(path: &Path) -> Result<Self, PublicationError> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                PublicationError::new(format!("{} has no UTF-8 file name", path.display()))
            })?
            .to_owned();
        let parsed = Filename::parse(&filename).ok_or_else(|| {
            PublicationError::new(format!(
                "{filename} is neither a wheel (.whl) nor an sdist (.tar.gz)"
            ))
        })?;
        let open = || {
            File::open(path).map_err(|error| {
                PublicationError::new(format!("could not read {}: {error}", path.display()))
            })
        };
        let sha256 = sha256_digest_reader(BufReader::new(open()?))
            .map_err(|error| {
                PublicationError::new(format!("could not read {}: {error}", path.display()))
            })?
            .trim_start_matches("sha256:")
            .to_owned();
        let text = match parsed.filetype {
            Filetype::Wheel => wheel_metadata(open()?, &parsed),
            Filetype::Sdist => sdist_metadata(open()?, &parsed),
        }
        .map_err(|error| PublicationError::new(format!("{filename}: {error}")))?;
        Ok(Self {
            filename,
            path: path.to_owned(),
            parsed,
            sha256,
            metadata: form_fields(&text),
        })
    }

    /// The first value of a metadata field, such as `version`.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads every distribution in `directory`, by filename. Every regular file must be one.
pub fn read_directory(directory: &Path) -> Result<Vec<Distribution>, PublicationError> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        PublicationError::new(format!(
            "could not read the distributions in {}: {error}",
            directory.display()
        ))
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| {
                PublicationError::new(format!(
                    "could not read the distributions in {}: {error}",
                    directory.display()
                ))
            })?
            .path();
        // `metadata` follows symlinks, as the store paths of `symlinkJoin` are. An entry that
        // cannot be read could be a distribution of the release, so it stops the run.
        let metadata = std::fs::metadata(&path).map_err(|error| {
            PublicationError::new(format!("could not read {}: {error}", path.display()))
        })?;
        if !metadata.is_file() {
            return Err(PublicationError::new(format!(
                "{} is not a regular file; the distributions must be files",
                path.display()
            )));
        }
        paths.push(path);
    }
    paths.sort();
    if paths.is_empty() {
        return Err(PublicationError::new(format!(
            "{} holds no distributions",
            directory.display()
        )));
    }
    paths.iter().map(|path| Distribution::read(path)).collect()
}

/// The `METADATA` file of a wheel, in its `{name}-{version}.dist-info` directory.
fn wheel_metadata(file: File, parsed: &Filename) -> Result<String, String> {
    let mut archive =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|error| error.to_string())?;
    let entry = format!("{}-{}.dist-info/METADATA", parsed.name, parsed.version);
    let mut metadata = archive
        .by_name(&entry)
        .map_err(|error| format!("could not read {entry}: {error}"))?;
    let mut text = String::new();
    metadata
        .read_to_string(&mut text)
        .map_err(|error| format!("could not read {entry}: {error}"))?;
    Ok(text)
}

/// The `PKG-INFO` file of an sdist, in its `{name}-{version}` directory.
fn sdist_metadata(file: File, parsed: &Filename) -> Result<String, String> {
    let entry = format!("{}-{}/PKG-INFO", parsed.name, parsed.version);
    let mut archive = tar::Archive::new(GzDecoder::new(BufReader::new(file)));
    for member in archive.entries().map_err(|error| error.to_string())? {
        let mut member = member.map_err(|error| error.to_string())?;
        let path = member.path().map_err(|error| error.to_string())?;
        if path.to_str().map(|path| path.trim_start_matches("./")) == Some(entry.as_str()) {
            let mut text = String::new();
            member
                .read_to_string(&mut text)
                .map_err(|error| format!("could not read {entry}: {error}"))?;
            return Ok(text);
        }
    }
    Err(format!("has no {entry}"))
}

/// The core metadata fields that the upload form carries, by form name. Other headers stay out,
/// so none can stand in for a field of the upload itself, such as `filetype`.
const CORE_FIELDS: &[&str] = &[
    "metadata_version",
    "name",
    "version",
    "dynamic",
    "platform",
    "supported_platform",
    "summary",
    "description",
    "description_content_type",
    "keywords",
    "home_page",
    "download_url",
    "author",
    "author_email",
    "maintainer",
    "maintainer_email",
    "license",
    "license_expression",
    "license_file",
    "classifiers",
    "requires_dist",
    "requires_python",
    "requires_external",
    "project_urls",
    "provides_extra",
    "provides_dist",
    "obsoletes_dist",
    "requires",
    "provides",
    "obsoletes",
];

/// Core metadata, in email header format, as upload form fields, like twine and uv send them:
/// each core metadata header in snake case, `Classifier` and `Project-URL` in their plural form
/// names, and the message body as `description`.
#[must_use]
pub fn form_fields(text: &str) -> Vec<(String, String)> {
    let text = text.replace("\r\n", "\n");
    let (headers, body) = text.split_once("\n\n").unwrap_or((&text, ""));
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in headers.lines() {
        if line.starts_with([' ', '\t']) {
            // A folded line continues the previous field; a long description keeps its lines,
            // each prefixed with up to eight spaces and a `|`.
            if let Some((_, value)) = fields.last_mut() {
                let continued = line.trim_start();
                let continued = continued.strip_prefix('|').unwrap_or(continued);
                value.push('\n');
                value.push_str(continued);
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let field = match name.trim().to_ascii_lowercase().as_str() {
            "classifier" => "classifiers".to_owned(),
            "project-url" => "project_urls".to_owned(),
            other => other.replace('-', "_"),
        };
        fields.push((field, value.trim().to_owned()));
    }
    let body = body.trim_end_matches('\n');
    if !body.is_empty() {
        fields.push(("description".to_owned(), body.to_owned()));
    }
    fields.retain(|(field, _)| CORE_FIELDS.contains(&field.as_str()));
    fields
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    pub(crate) const METADATA: &str = "Metadata-Version: 2.4\nName: My.Package\nVersion: 1.2.3\nSummary: A test package\nClassifier: Programming Language :: Python :: 3\nClassifier: Typing :: Typed\nProject-URL: Source, https://example.com/source\nDescription-Content-Type: text/markdown\n\n# My package\n\nIt does things.\n";

    /// A wheel holding `metadata` as its `METADATA`.
    pub(crate) fn wheel(directory: &Path, filename: &str, metadata: &str) -> PathBuf {
        let parsed = Filename::parse(filename).unwrap();
        let path = directory.join(filename);
        let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive
            .start_file("my_package/__init__.py", options)
            .unwrap();
        archive.write_all(b"").unwrap();
        archive
            .start_file(
                format!("{}-{}.dist-info/METADATA", parsed.name, parsed.version),
                options,
            )
            .unwrap();
        archive.write_all(metadata.as_bytes()).unwrap();
        archive.finish().unwrap();
        path
    }

    /// An sdist holding `metadata` as its `PKG-INFO`.
    pub(crate) fn sdist(directory: &Path, filename: &str, metadata: &str) -> PathBuf {
        let parsed = Filename::parse(filename).unwrap();
        let path = directory.join(filename);
        let encoder = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let root = format!("{}-{}", parsed.name, parsed.version);
        for (entry, contents) in [
            (format!("{root}/pyproject.toml"), b"[project]\n".as_slice()),
            (format!("{root}/PKG-INFO"), metadata.as_bytes()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(1);
            archive.append_data(&mut header, entry, contents).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn reads_wheel_and_sdist_filenames() {
        assert_eq!(
            Filename::parse("my_package-1.2.3-py3-none-any.whl"),
            Some(Filename {
                name: "my_package".into(),
                version: "1.2.3".into(),
                filetype: Filetype::Wheel,
                pyversion: "py3".into(),
            })
        );
        assert_eq!(
            Filename::parse("my_package-1.2.3-1-cp312-cp312-manylinux_2_17_x86_64.whl")
                .unwrap()
                .pyversion,
            "cp312"
        );
        assert_eq!(
            Filename::parse("my_package-1.2.3.tar.gz"),
            Some(Filename {
                name: "my_package".into(),
                version: "1.2.3".into(),
                filetype: Filetype::Sdist,
                pyversion: "source".into(),
            })
        );
        for other in [
            "my_package-1.2.3.zip",
            "my_package-1.2.3-py3-any.whl",
            "my_package.tar.gz",
            "README.md",
        ] {
            assert_eq!(Filename::parse(other), None, "{other}");
        }
    }

    #[test]
    fn normalizes_project_names() {
        for name in ["My.Package", "my_package", "my--package", "MY-_.PACKAGE"] {
            assert_eq!(normalize(name), "my-package", "{name}");
        }
    }

    #[test]
    fn forwards_metadata_as_form_fields() {
        let text = "Metadata-Version: 2.1\r\nName: legacy\r\nVersion: 1.0.0\r\nDescription: First line\r\n        |  indented\r\n        |\r\n        |last\r\nRequires-Dist: requests\r\nFiletype: sdist\r\nSha256-Digest: 00\r\n  folded\r\nX-Custom: kept out\r\nRequires-Dist: rich; extra == \"cli\"\r\n";
        assert_eq!(
            form_fields(text),
            [
                ("metadata_version", "2.1"),
                ("name", "legacy"),
                ("version", "1.0.0"),
                ("description", "First line\n  indented\n\nlast"),
                ("requires_dist", "requests"),
                ("requires_dist", "rich; extra == \"cli\""),
            ]
            .map(|(field, value)| (field.to_owned(), value.to_owned()))
        );
        let fields = form_fields(METADATA);
        assert!(fields.contains(&("classifiers".into(), "Typing :: Typed".into())));
        assert!(fields.contains(&(
            "project_urls".into(),
            "Source, https://example.com/source".into()
        )));
        assert_eq!(
            fields.last().unwrap(),
            &(
                "description".into(),
                "# My package\n\nIt does things.".into()
            )
        );
    }

    #[test]
    fn reads_the_metadata_of_wheels_and_sdists() {
        let directory = tempfile::tempdir().unwrap();
        wheel(
            directory.path(),
            "my_package-1.2.3-py3-none-any.whl",
            METADATA,
        );
        sdist(directory.path(), "my_package-1.2.3.tar.gz", METADATA);
        let distributions = read_directory(directory.path()).unwrap();
        assert_eq!(
            distributions
                .iter()
                .map(|distribution| distribution.filename.as_str())
                .collect::<Vec<_>>(),
            [
                "my_package-1.2.3-py3-none-any.whl",
                "my_package-1.2.3.tar.gz"
            ]
        );
        for distribution in &distributions {
            assert_eq!(distribution.field("name"), Some("My.Package"));
            assert_eq!(distribution.field("version"), Some("1.2.3"));
            assert_eq!(distribution.sha256.len(), 64);
            assert_eq!(
                distribution.sha256,
                sha256_digest_reader(File::open(&distribution.path).unwrap())
                    .unwrap()
                    .trim_start_matches("sha256:")
            );
        }
    }

    #[test]
    fn every_file_must_be_a_distribution() {
        let directory = tempfile::tempdir().unwrap();
        let error = read_directory(directory.path()).unwrap_err().to_string();
        assert!(error.ends_with("holds no distributions"), "{error}");

        std::fs::write(directory.path().join("README.md"), "").unwrap();
        let error = read_directory(directory.path()).unwrap_err().to_string();
        assert_eq!(
            error,
            "README.md is neither a wheel (.whl) nor an sdist (.tar.gz)"
        );

        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("my_package-1.2.3-py3-none-any.whl"),
            "not a zip",
        )
        .unwrap();
        let error = read_directory(directory.path()).unwrap_err().to_string();
        assert!(
            error.starts_with("my_package-1.2.3-py3-none-any.whl: "),
            "{error}"
        );

        // A wheel that cannot be read is never left out.
        let directory = tempfile::tempdir().unwrap();
        sdist(directory.path(), "my_package-1.2.3.tar.gz", METADATA);
        std::os::unix::fs::symlink(
            directory.path().join("missing"),
            directory.path().join("my_package-1.2.3-py3-none-any.whl"),
        )
        .unwrap();
        let error = read_directory(directory.path()).unwrap_err().to_string();
        assert!(error.starts_with("could not read "), "{error}");
        assert!(
            error.contains("my_package-1.2.3-py3-none-any.whl"),
            "{error}"
        );

        let directory = tempfile::tempdir().unwrap();
        sdist(directory.path(), "my_package-1.2.3.tar.gz", METADATA);
        std::fs::create_dir(directory.path().join("nested")).unwrap();
        let error = read_directory(directory.path()).unwrap_err().to_string();
        assert!(
            error.ends_with("nested is not a regular file; the distributions must be files"),
            "{error}"
        );
    }
}
