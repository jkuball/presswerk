use presswerk::changelog::{ChangelogError, insert_version_section, read_version_section};
use semver::Version;

#[test]
fn creates_a_keep_a_changelog_file_and_reads_the_frozen_notes() {
    let version = Version::new(1, 2, 0);
    let notes = "### Features\n\n- add safe release commits\n";

    let changelog = insert_version_section(None, &version, "2026-11-04", notes).unwrap();

    assert_eq!(
        changelog,
        "# Changelog\n\n## [1.2.0] - 2026-11-04\n\n### Features\n\n- add safe release commits\n\n"
    );
    assert_eq!(read_version_section(&changelog, &version).unwrap(), notes);
}

#[test]
fn inserts_the_newest_version_without_changing_older_sections() {
    let old =
        "# Changelog\n\nProject history.\n\n## [1.1.0] - 2026-10-01\n\n### Fixes\n\n- old fix\n";
    let version = Version::new(1, 2, 0);

    let changelog = insert_version_section(
        Some(old),
        &version,
        "2026-11-04",
        "### Features\n\n- new feature\n",
    )
    .unwrap();

    assert!(changelog.starts_with("# Changelog\n\nProject history.\n\n## [1.2.0]"));
    assert!(changelog.ends_with("## [1.1.0] - 2026-10-01\n\n### Fixes\n\n- old fix\n"));
    assert_eq!(
        read_version_section(&changelog, &Version::new(1, 1, 0)).unwrap(),
        "### Fixes\n\n- old fix"
    );
}

#[test]
fn leaves_an_optional_unreleased_section_above_releases() {
    let contents = "# Changelog\n\n## [Unreleased]\n\nManual notes.\n\n## [1.0.0] - 2026-01-01\n\nFirst release.\n";

    let changelog = insert_version_section(
        Some(contents),
        &Version::new(1, 1, 0),
        "2026-11-04",
        "Next release.\n",
    )
    .unwrap();

    assert!(changelog.contains("## [Unreleased]\n\nManual notes.\n\n## [1.1.0] - 2026-11-04"));
}

#[test]
fn round_trips_crlf_notes_without_changing_their_bytes() {
    let old_version = Version::new(1, 0, 0);
    let old_notes = "### Fixes\r\n\r\n- preserve old notes\r\n";
    let old = insert_version_section(None, &old_version, "2026-10-01", old_notes).unwrap();
    let new_version = Version::new(1, 1, 0);
    let new_notes = "### Features\r\n\r\n- preserve new notes\r\n";

    let changelog =
        insert_version_section(Some(&old), &new_version, "2026-11-04", new_notes).unwrap();

    assert!(changelog.starts_with("# Changelog\r\n\r\n## [1.1.0]"));
    assert_eq!(
        read_version_section(&changelog, &new_version).unwrap(),
        new_notes
    );
    assert_eq!(
        read_version_section(&changelog, &old_version).unwrap(),
        old_notes
    );
}

#[test]
fn rejects_duplicate_and_missing_versions() {
    let version = Version::new(1, 0, 0);
    let changelog = insert_version_section(None, &version, "2026-11-04", "First.").unwrap();

    assert!(matches!(
        insert_version_section(Some(&changelog), &version, "2026-11-04", "Again.")
            .unwrap_err(),
        ChangelogError::DuplicateVersion(found) if found == version
    ));
    assert!(matches!(
        read_version_section(&changelog, &Version::new(2, 0, 0)).unwrap_err(),
        ChangelogError::MissingVersion(found) if found == Version::new(2, 0, 0)
    ));
}

#[test]
fn rejects_duplicate_versions_when_reading() {
    let version = Version::new(1, 0, 0);
    let changelog = insert_version_section(None, &version, "2026-11-04", "First.\n").unwrap();
    let duplicate = format!("{changelog}{changelog}");

    assert!(matches!(
        read_version_section(&duplicate, &version).unwrap_err(),
        ChangelogError::DuplicateVersion(found) if found == version
    ));
}
