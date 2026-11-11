use presswerk::declaration::{
    CargoLockVersionFile, JsonVersionFile, PlainOptions, PlainVersionFile, RegexVersionFile,
    TomlVersionFile,
};
use presswerk::version::{VersionFileWriter, check_consistent_versions};
use semver::Version;

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
