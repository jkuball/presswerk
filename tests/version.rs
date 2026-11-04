use presswerk::declaration::{CargoLockVersionFile, TomlVersionFile};
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
