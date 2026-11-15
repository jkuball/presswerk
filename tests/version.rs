use presswerk::declaration::{
    CargoLockVersionFile, JsonVersionFile, PlainOptions, PlainVersionFile, RegexVersionFile,
    TomlVersionFile,
};
use presswerk::version::{
    CargoWorkspaceLock, SourceTree, VersionFileError, VersionFileWriter, cargo_workspace_packages,
    check_consistent_versions,
};
use semver::Version;
use std::collections::BTreeMap;

#[test]
fn writes_and_reads_a_toml_version_without_reformatting_the_file() {
    let writer = TomlVersionFile {
        path: "Cargo.toml".into(),
        toml: "package.version".into(),
    };
    let contents = "# manifest\n[package]\nname = \"example\"\nversion = \"1.2.3\" # released\n\n[dependencies]\nserde = \"1\"\n";

    let written = writer
        .write_version(contents, &Version::new(1, 3, 0))
        .unwrap();

    assert_eq!(
        writer.read_version(&written).unwrap(),
        Version::new(1, 3, 0)
    );
    assert!(written.starts_with("# manifest\n[package]\nname = \"example\"\n"));
    assert!(written.contains("version = \"1.3.0\" # released"));
    assert!(written.ends_with("[dependencies]\nserde = \"1\"\n"));
}

#[test]
fn preserves_crlf_around_a_toml_version() {
    let writer = TomlVersionFile {
        path: "Cargo.toml".into(),
        toml: "package.version".into(),
    };
    let contents = "[package]\r\nname = \"example\"\r\nversion = \"1.2.3\" # released\r\n\r\n[dependencies]\r\nserde = \"1\"\r\n";

    let written = writer
        .write_version(contents, &Version::new(1, 3, 0))
        .unwrap();

    assert_eq!(written, contents.replace("\"1.2.3\"", "\"1.3.0\""));
}

#[test]
fn writes_only_the_named_cargo_lock_package() {
    let writer = CargoLockVersionFile {
        path: "Cargo.lock".into(),
        cargo_lock: "example".into(),
    };
    let contents = r#"version = 4

[[package]]
name = "dependency"
version = "9.9.9"

[[package]]
name = "example"
version = "1.2.3"
dependencies = [
 "dependency",
]
"#;

    let written = writer
        .write_version(contents, &Version::new(2, 0, 0))
        .unwrap();

    assert_eq!(
        writer.read_version(&written).unwrap(),
        Version::new(2, 0, 0)
    );
    assert!(written.contains("name = \"dependency\"\nversion = \"9.9.9\""));
    assert!(written.contains("name = \"example\"\nversion = \"2.0.0\""));
}

#[test]
fn preserves_crlf_around_a_cargo_lock_version() {
    let writer = CargoLockVersionFile {
        path: "Cargo.lock".into(),
        cargo_lock: "example".into(),
    };
    let contents = "version = 4\r\n\r\n[[package]]\r\nname = \"dependency\"\r\nversion = \"9.9.9\"\r\n\r\n[[package]]\r\nname = \"example\"\r\nversion = \"1.2.3\"\r\n";
    let expected = contents.replace(
        "name = \"example\"\r\nversion = \"1.2.3\"",
        "name = \"example\"\r\nversion = \"2.0.0\"",
    );

    let written = writer
        .write_version(contents, &Version::new(2, 0, 0))
        .unwrap();

    assert_eq!(written, expected);
}

#[test]
fn requires_one_unambiguous_cargo_lock_package() {
    let writer = CargoLockVersionFile {
        path: "Cargo.lock".into(),
        cargo_lock: "duplicate".into(),
    };
    let contents = r#"version = 4

[[package]]
name = "duplicate"
version = "1.0.0"

[[package]]
name = "duplicate"
version = "2.0.0"
"#;

    assert!(
        writer
            .read_version(contents)
            .unwrap_err()
            .to_string()
            .contains("more than one package")
    );
}

#[test]
fn checks_that_all_version_files_agree() {
    let manifest = TomlVersionFile {
        path: "Cargo.toml".into(),
        toml: "package.version".into(),
    };
    let lock = CargoLockVersionFile {
        path: "Cargo.lock".into(),
        cargo_lock: "example".into(),
    };
    let manifest_contents = "[package]\nversion = \"1.2.3\"\n";
    let matching_lock = "[[package]]\nname = \"example\"\nversion = \"1.2.3\"\n";
    let mismatching_lock = matching_lock.replace("1.2.3", "1.2.4");

    assert_eq!(
        check_consistent_versions(&[
            (&manifest as &dyn VersionFileWriter, manifest_contents),
            (&lock as &dyn VersionFileWriter, matching_lock),
        ])
        .unwrap(),
        Version::new(1, 2, 3)
    );
    assert!(
        check_consistent_versions(&[
            (&manifest as &dyn VersionFileWriter, manifest_contents),
            (&lock as &dyn VersionFileWriter, &mismatching_lock),
        ])
        .unwrap_err()
        .to_string()
        .contains("version files disagree")
    );
}

#[test]
fn writes_a_nested_json_version_without_reformatting_the_file() {
    let writer = JsonVersionFile {
        path: "plugin.json".into(),
        json: "plugin.version".into(),
    };
    let contents = "{\n    \"name\": \"example\",\n    \"plugin\": { \"version\": \"1.2.3\", \"tags\": [\"a\"] },\n    \"version\": \"9.9.9\"\n}\n";

    let written = writer
        .write_version(contents, &Version::new(1, 3, 0))
        .unwrap();

    assert_eq!(written, contents.replace("\"1.2.3\"", "\"1.3.0\""));
    assert_eq!(
        writer.read_version(&written).unwrap(),
        Version::new(1, 3, 0)
    );
}

#[test]
fn writes_the_root_package_version_of_an_npm_lockfile() {
    let writer = JsonVersionFile {
        path: "package-lock.json".into(),
        json: r#"packages."".version"#.into(),
    };
    let contents = r#"{
  "name": "frontend",
  "version": "0.38.2",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "frontend",
      "version": "0.38.2"
    },
    "node_modules/left-pad": {
      "version": "0.38.2"
    }
  }
}
"#;
    assert_eq!(
        writer.read_version(contents).unwrap(),
        Version::new(0, 38, 2)
    );

    let written = writer
        .write_version(contents, &Version::new(0, 39, 0))
        .unwrap();

    assert_eq!(
        written,
        contents.replacen(
            "\"name\": \"frontend\",\n      \"version\": \"0.38.2\"",
            "\"name\": \"frontend\",\n      \"version\": \"0.39.0\"",
            1
        )
    );
}

#[test]
fn selects_a_toml_key_segment_with_a_dot() {
    let writer = TomlVersionFile {
        path: "versions.toml".into(),
        toml: r#"components."api.v1".version"#.into(),
    };
    let contents = "[components.\"api.v1\"]\nversion = \"1.2.3\"\n\n[components.api.v1]\nversion = \"9.9.9\"\n";

    let written = writer
        .write_version(contents, &Version::new(1, 3, 0))
        .unwrap();

    assert_eq!(written, contents.replace("\"1.2.3\"", "\"1.3.0\""));
}

#[test]
fn rejects_a_malformed_key() {
    let writer = JsonVersionFile {
        path: "package-lock.json".into(),
        json: r#"packages."version"#.into(),
    };
    assert!(
        writer
            .read_version("{}")
            .unwrap_err()
            .to_string()
            .contains("is not a TOML dotted key")
    );
}

#[test]
fn rejects_missing_and_non_string_json_versions() {
    let missing = JsonVersionFile {
        path: "package.json".into(),
        json: "version".into(),
    };
    assert!(
        missing
            .read_version("{\"name\": \"example\"}")
            .unwrap_err()
            .to_string()
            .contains("does not contain JSON key version")
    );
    assert!(
        missing
            .write_version("{\"version\": 1}", &Version::new(1, 0, 0))
            .unwrap_err()
            .to_string()
            .contains("must be a string")
    );
}

#[test]
fn keeps_the_line_break_of_a_plain_version_file() {
    let writer = PlainVersionFile {
        path: "VERSION".into(),
        plain: PlainOptions {},
    };

    for (contents, written) in [
        ("1.2.3\n", "1.3.0\n"),
        ("1.2.3\r\n", "1.3.0\r\n"),
        ("1.2.3", "1.3.0"),
    ] {
        assert_eq!(
            writer.read_version(contents).unwrap(),
            Version::new(1, 2, 3)
        );
        assert_eq!(
            writer
                .write_version(contents, &Version::new(1, 3, 0))
                .unwrap(),
            written
        );
    }
    assert!(writer.read_version("1.2.3\nmore\n").is_err());
}

#[test]
fn replaces_only_the_version_group_of_a_regex_match() {
    let writer = RegexVersionFile {
        path: "Chart.yaml".into(),
        regex: r"^version: (?<version>\S+)$".into(),
    };
    let contents = "apiVersion: v2\nversion: 1.2.3\nappVersion: 1.2.3\n";

    let written = writer
        .write_version(contents, &Version::new(1, 3, 0))
        .unwrap();

    assert_eq!(
        written,
        "apiVersion: v2\nversion: 1.3.0\nappVersion: 1.2.3\n"
    );
    assert_eq!(
        writer.read_version(&written).unwrap(),
        Version::new(1, 3, 0)
    );
}

#[test]
fn a_regex_must_match_exactly_once() {
    let writer = RegexVersionFile {
        path: "Chart.yaml".into(),
        regex: r"(?<version>\d+\.\d+\.\d+)".into(),
    };

    for (contents, message) in [
        ("no version here\n", "does not match"),
        ("1.2.3 and 4.5.6\n", "matches more than once"),
    ] {
        assert!(
            writer
                .read_version(contents)
                .unwrap_err()
                .to_string()
                .contains(message)
        );
    }
}

/// A commit made of the given files.
struct Files(BTreeMap<&'static str, &'static str>);

impl SourceTree for Files {
    fn read(&self, path: &str) -> Result<Option<String>, VersionFileError> {
        Ok(self.0.get(path).map(|contents| (*contents).to_owned()))
    }

    fn directories(&self) -> Result<Vec<String>, VersionFileError> {
        let mut directories: Vec<String> = self
            .0
            .keys()
            .flat_map(|path| {
                let parts: Vec<&str> = path.split('/').collect();
                (1..parts.len()).map(move |end| parts[..end].join("/"))
            })
            .collect();
        directories.sort();
        directories.dedup();
        Ok(directories)
    }
}

fn member(name: &str, version: &str) -> String {
    format!("[package]\nname = \"{name}\"\n{version}\n")
}

#[test]
fn resolves_the_workspace_members_that_inherit_the_version() {
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let files = Files(BTreeMap::from([
        (
            "rust/Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\", \"./tools/cli/\", \"crates/*\"]\nexclude = [\"crates/old\", \"tools\"]\n\n[workspace.package]\nversion = \"1.2.3\"\n\n[package]\nname = \"root\"\nversion.workspace = true\n",
        ),
        (
            "rust/crates/a/Cargo.toml",
            leak(member("a", "version.workspace = true")),
        ),
        (
            "rust/crates/b/Cargo.toml",
            leak(member("b", "version = { workspace = true }")),
        ),
        (
            "rust/crates/own/Cargo.toml",
            leak(member("own", "version = \"9.0.0\"")),
        ),
        (
            "rust/crates/old/Cargo.toml",
            leak(member("old", "version.workspace = true")),
        ),
        (
            "rust/crates/a/nested/Cargo.toml",
            leak(member("nested", "version.workspace = true")),
        ),
        (
            "rust/tools/cli/Cargo.toml",
            leak(member("cli", "version.workspace = true")),
        ),
    ]));

    assert_eq!(
        cargo_workspace_packages("rust/Cargo.toml", &files).unwrap(),
        ["root", "a", "b", "cli"]
    );
}

#[test]
fn rejects_workspaces_without_inheriting_members_or_with_missing_manifests() {
    let own_versions = Files(BTreeMap::from([
        ("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n"),
        (
            "a/Cargo.toml",
            "[package]\nname = \"a\"\nversion = \"1.0.0\"\n",
        ),
    ]));
    assert!(
        cargo_workspace_packages("Cargo.toml", &own_versions)
            .unwrap_err()
            .to_string()
            .contains("inherits workspace.package.version")
    );

    let missing = Files(BTreeMap::from([(
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\"]\n",
    )]));
    assert!(
        cargo_workspace_packages("Cargo.toml", &missing)
            .unwrap_err()
            .to_string()
            .contains("workspace member a of Cargo.toml has no Cargo.toml")
    );

    let crate_manifest = Files(BTreeMap::from([(
        "Cargo.toml",
        "[package]\nname = \"a\"\nversion = \"1.0.0\"\n",
    )]));
    assert!(
        cargo_workspace_packages("Cargo.toml", &crate_manifest)
            .unwrap_err()
            .to_string()
            .contains("has no [workspace] table")
    );
}

#[test]
fn writes_and_reads_every_workspace_member_in_cargo_lock() {
    let writer = CargoWorkspaceLock {
        path: "Cargo.lock".into(),
        packages: vec!["a".into(), "b".into()],
    };
    let contents = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.2.3\"\n\n[[package]]\nname = \"dependency\"\nversion = \"1.2.3\"\n\n[[package]]\nname = \"b\"\nversion = \"1.2.3\"\ndependencies = [\n \"a\",\n]\n";

    let written = writer
        .write_version(contents, &Version::new(1, 10, 0))
        .unwrap();

    assert_eq!(
        written,
        contents.replacen("\"1.2.3\"", "\"1.10.0\"", 1).replace(
            "name = \"b\"\nversion = \"1.2.3\"",
            "name = \"b\"\nversion = \"1.10.0\""
        )
    );
    assert_eq!(
        writer.read_version(&written).unwrap(),
        Version::new(1, 10, 0)
    );
    assert!(
        writer
            .read_version(contents.replacen("1.2.3", "1.2.4", 1).as_str())
            .unwrap_err()
            .to_string()
            .contains("workspace members disagree in Cargo.lock: a has 1.2.4, but b has 1.2.3")
    );
}
