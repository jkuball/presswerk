use presswerk::changelog::{
    ChangelogError, ChangelogHeading, insert_version_section, read_version_section,
};
use semver::Version;

#[test]
fn creates_a_keep_a_changelog_file_and_reads_the_frozen_notes() {
    let version = Version::new(1, 2, 0);
    let notes = "### Features\n\n- add safe release commits\n";

    let changelog = insert_version_section(
        None,
        &ChangelogHeading::default(),
        &version,
        "2026-11-04",
        notes,
    )
    .unwrap();

    assert_eq!(
        changelog,
        "# Changelog\n\n## [1.2.0] - 2026-11-04\n\n### Features\n\n- add safe release commits\n\n"
    );
    assert_eq!(
        read_version_section(&changelog, &ChangelogHeading::default(), &version).unwrap(),
        notes
    );
}

#[test]
fn inserts_the_newest_version_without_changing_older_sections() {
    let old =
        "# Changelog\n\nProject history.\n\n## [1.1.0] - 2026-10-01\n\n### Fixes\n\n- old fix\n";
    let version = Version::new(1, 2, 0);

    let changelog = insert_version_section(
        Some(old),
        &ChangelogHeading::default(),
        &version,
        "2026-11-04",
        "### Features\n\n- new feature\n",
    )
    .unwrap();

    assert!(changelog.starts_with("# Changelog\n\nProject history.\n\n## [1.2.0]"));
    assert!(changelog.ends_with("## [1.1.0] - 2026-10-01\n\n### Fixes\n\n- old fix\n"));
    assert_eq!(
        read_version_section(
            &changelog,
            &ChangelogHeading::default(),
            &Version::new(1, 1, 0)
        )
        .unwrap(),
        "### Fixes\n\n- old fix"
    );
}

#[test]
fn leaves_an_optional_unreleased_section_above_releases() {
    let contents = "# Changelog\n\n## [Unreleased]\n\nManual notes.\n\n## [1.0.0] - 2026-01-01\n\nFirst release.\n";

    let changelog = insert_version_section(
        Some(contents),
        &ChangelogHeading::default(),
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
    let old = insert_version_section(
        None,
        &ChangelogHeading::default(),
        &old_version,
        "2026-10-01",
        old_notes,
    )
    .unwrap();
    let new_version = Version::new(1, 1, 0);
    let new_notes = "### Features\r\n\r\n- preserve new notes\r\n";

    let changelog = insert_version_section(
        Some(&old),
        &ChangelogHeading::default(),
        &new_version,
        "2026-11-04",
        new_notes,
    )
    .unwrap();

    assert!(changelog.starts_with("# Changelog\r\n\r\n## [1.1.0]"));
    assert_eq!(
        read_version_section(&changelog, &ChangelogHeading::default(), &new_version).unwrap(),
        new_notes
    );
    assert_eq!(
        read_version_section(&changelog, &ChangelogHeading::default(), &old_version).unwrap(),
        old_notes
    );
}

#[test]
fn rejects_duplicate_and_missing_versions() {
    let version = Version::new(1, 0, 0);
    let changelog = insert_version_section(
        None,
        &ChangelogHeading::default(),
        &version,
        "2026-11-04",
        "First.",
    )
    .unwrap();

    assert!(matches!(
        insert_version_section(Some(&changelog), &ChangelogHeading::default(), &version, "2026-11-04", "Again.")
            .unwrap_err(),
        ChangelogError::DuplicateVersion(found) if found == version
    ));
    assert!(matches!(
        read_version_section(&changelog, &ChangelogHeading::default(), &Version::new(2, 0, 0)).unwrap_err(),
        ChangelogError::MissingVersion(found) if found == Version::new(2, 0, 0)
    ));
}

#[test]
fn rejects_duplicate_versions_when_reading() {
    let version = Version::new(1, 0, 0);
    let changelog = insert_version_section(
        None,
        &ChangelogHeading::default(),
        &version,
        "2026-11-04",
        "First.\n",
    )
    .unwrap();
    let duplicate = format!("{changelog}{changelog}");

    assert!(matches!(
        read_version_section(&duplicate, &ChangelogHeading::default(), &version).unwrap_err(),
        ChangelogError::DuplicateVersion(found) if found == version
    ));
}

#[test]
fn inserts_before_the_first_level_two_heading_of_another_format() {
    let old = "# CHANGELOG\n\nIntro.\n\n## v0.38.2 (2026-11-10)\n\n- old fix\n\n## v0.38.1 (2026-11-01)\n\n- older fix\n";

    let changelog = insert_version_section(
        Some(old),
        &ChangelogHeading::default(),
        &Version::new(0, 39, 0),
        "2026-11-20",
        "- new feature\n",
    )
    .unwrap();

    assert_eq!(
        changelog,
        "# CHANGELOG\n\nIntro.\n\n## [0.39.0] - 2026-11-20\n\n- new feature\n\n## v0.38.2 (2026-11-10)\n\n- old fix\n\n## v0.38.1 (2026-11-01)\n\n- older fix\n"
    );
    assert_eq!(
        read_version_section(
            &changelog,
            &ChangelogHeading::default(),
            &Version::new(0, 39, 0)
        )
        .unwrap(),
        "- new feature\n"
    );
}

#[test]
fn appends_when_the_changelog_has_no_level_two_heading() {
    let changelog = insert_version_section(
        Some("# Changelog\n\nNothing yet.\n"),
        &ChangelogHeading::default(),
        &Version::new(1, 0, 0),
        "2026-11-20",
        "First.\n",
    )
    .unwrap();

    assert_eq!(
        changelog,
        "# Changelog\n\nNothing yet.\n\n## [1.0.0] - 2026-11-20\n\nFirst.\n\n"
    );
}

#[test]
fn renders_and_finds_headings_in_a_declared_format() {
    let heading = ChangelogHeading::parse("## v{version} ({date})").unwrap();
    let old = "# CHANGELOG\n\n## v0.38.2 (2026-11-10)\n\n### Fix\n\n- old fix\n\n## v0.38.1 (2026-11-01)\n\n- older fix\n";

    let changelog = insert_version_section(
        Some(old),
        &heading,
        &Version::new(0, 39, 0),
        "2026-11-20",
        "### Feature\n\n- new feature\n",
    )
    .unwrap();

    assert!(changelog.starts_with(
        "# CHANGELOG\n\n## v0.39.0 (2026-11-20)\n\n### Feature\n\n- new feature\n\n## v0.38.2 (2026-11-10)"
    ));
    assert_eq!(
        read_version_section(&changelog, &heading, &Version::new(0, 38, 2)).unwrap(),
        "### Fix\n\n- old fix\n"
    );
    assert!(matches!(
        insert_version_section(
            Some(&changelog),
            &heading,
            &Version::new(0, 38, 1),
            "2026-11-21",
            "Again.\n",
        )
        .unwrap_err(),
        ChangelogError::DuplicateVersion(_)
    ));
}

#[test]
fn finds_headings_with_the_date_first_or_without_a_date() {
    let date_first = ChangelogHeading::parse("## {date}: Release {version}").unwrap();
    let changelog = "## 2026-11-10: Release 1.0.0-rc.1 \n\nNotes.\n";
    assert_eq!(
        read_version_section(
            changelog,
            &date_first,
            &Version::parse("1.0.0-rc.1").unwrap()
        )
        .unwrap(),
        "Notes."
    );

    let without_date = ChangelogHeading::parse("### {version}").unwrap();
    let changelog = insert_version_section(
        Some("# Changelog\n\n### 1.0.0\n\nFirst.\n"),
        &without_date,
        &Version::new(1, 1, 0),
        "2026-11-20",
        "#### Features\n\n- second\n",
    )
    .unwrap();
    assert_eq!(
        changelog,
        "# Changelog\n\n### 1.1.0\n\n#### Features\n\n- second\n\n### 1.0.0\n\nFirst.\n"
    );
    assert_eq!(
        read_version_section(&changelog, &without_date, &Version::new(1, 1, 0)).unwrap(),
        "#### Features\n\n- second\n"
    );
}

#[test]
fn rejects_invalid_heading_formats() {
    for format in [
        "## [{version}] - {date}",
        "# {version}",
        "## {date} {version}",
        "## {{version}}",
    ] {
        ChangelogHeading::parse(format).unwrap();
    }
    for format in [
        "Release {version}",
        "## no version",
        "## {version} {version}",
        "## {version} {date} {date}",
        "## {version}\n{date}",
    ] {
        assert!(matches!(
            ChangelogHeading::parse(format).unwrap_err(),
            ChangelogError::InvalidHeading(_)
        ));
    }
}

#[test]
fn round_trips_headings_with_trailing_space_and_adjacent_or_hyphenated_dates() {
    let prerelease = Version::parse("1.2.3-rc.1").unwrap();
    let stable = Version::new(1, 2, 3);
    for format in [
        "## {version} ",
        "## {version}-{date}",
        "## {version}{date}",
        "## {date}{version}",
    ] {
        let heading = ChangelogHeading::parse(format).unwrap();
        let changelog =
            insert_version_section(None, &heading, &prerelease, "2026-11-04", "Candidate.\n")
                .unwrap();
        assert_eq!(
            read_version_section(&changelog, &heading, &prerelease).unwrap(),
            "Candidate.\n",
            "{format}"
        );

        let changelog = insert_version_section(
            Some(&changelog),
            &heading,
            &stable,
            "2026-11-05",
            "Stable.\n",
        )
        .unwrap();
        assert_eq!(
            read_version_section(&changelog, &heading, &stable).unwrap(),
            "Stable.\n",
            "{format}"
        );
        assert_eq!(
            read_version_section(&changelog, &heading, &prerelease).unwrap(),
            "Candidate.\n",
            "{format}"
        );
    }
}

#[test]
fn rejects_dates_that_headings_cannot_find_again() {
    for date in ["", "today", "2026-11-4", "2026-11-04\n"] {
        assert!(matches!(
            insert_version_section(
                None,
                &ChangelogHeading::default(),
                &Version::new(1, 0, 0),
                date,
                "Notes.\n",
            )
            .unwrap_err(),
            ChangelogError::InvalidDate(_)
        ));
    }
}

#[test]
fn refuses_notes_that_do_not_read_back_under_the_heading() {
    let heading = ChangelogHeading::parse("### v{version} ({date})").unwrap();

    assert!(matches!(
        insert_version_section(
            None,
            &heading,
            &Version::new(1, 0, 0),
            "2026-11-04",
            "### Features\n\n- new feature\n",
        )
        .unwrap_err(),
        ChangelogError::UnreadableSection(version) if version == Version::new(1, 0, 0)
    ));
}

#[test]
fn ends_a_section_at_a_higher_level_heading() {
    let heading = ChangelogHeading::parse("### v{version}").unwrap();
    let changelog = insert_version_section(
        Some("# Changelog\n\n## Older history\n\nWritten by hand.\n"),
        &heading,
        &Version::new(1, 0, 0),
        "2026-11-04",
        "- first\n",
    )
    .unwrap();

    assert_eq!(
        changelog,
        "# Changelog\n\n### v1.0.0\n\n- first\n\n## Older history\n\nWritten by hand.\n"
    );
    assert_eq!(
        read_version_section(&changelog, &heading, &Version::new(1, 0, 0)).unwrap(),
        "- first\n"
    );
}

#[test]
fn ignores_headings_inside_fenced_code_blocks() {
    let heading = ChangelogHeading::default();
    let notes = "Example:\n\n```markdown\n## [0.9.0] - 2026-01-01\n## Not a section\n```\n\n~~~~\n# comment\n~~~\n## still code\n~~~~\n";

    let changelog = insert_version_section(
        Some("# Changelog\n\n```\n## [Example]\n```\n\n## v0.1.0 (2026-01-01)\n\nOld.\n"),
        &heading,
        &Version::new(1, 0, 0),
        "2026-11-04",
        notes,
    )
    .unwrap();

    assert!(changelog.starts_with(
        "# Changelog\n\n```\n## [Example]\n```\n\n## [1.0.0] - 2026-11-04\n\nExample:"
    ));
    assert_eq!(
        read_version_section(&changelog, &heading, &Version::new(1, 0, 0)).unwrap(),
        notes
    );
    assert!(matches!(
        read_version_section(&changelog, &heading, &Version::new(0, 9, 0)).unwrap_err(),
        ChangelogError::MissingVersion(_)
    ));
}

#[test]
fn does_not_take_inline_code_for_a_code_fence() {
    let heading = ChangelogHeading::default();
    let changelog = "# Changelog\n\n## [1.0.0] - 2026-11-04\n\n```presswerk plan``` now prints more.\n\n## [0.9.0] - 2026-10-01\n\nOlder.\n";

    assert_eq!(
        read_version_section(changelog, &heading, &Version::new(1, 0, 0)).unwrap(),
        "```presswerk plan``` now prints more.\n"
    );
    assert_eq!(
        read_version_section(changelog, &heading, &Version::new(0, 9, 0)).unwrap(),
        "Older."
    );
}

#[test]
fn renders_and_finds_headings_with_a_declared_date_format() {
    let prerelease = Version::parse("1.2.3-rc.1").unwrap();
    let stable = Version::new(1, 2, 3);
    for (format, rendered) in [
        ("## v{version} ({date:%Y-%m-%d})", "## v1.2.3 (2026-11-05)"),
        (
            "## {version} vom {date:%d.%m.%Y}",
            "## 1.2.3 vom 05.11.2026",
        ),
        (
            "## {version} - {date:%B %e, %Y}",
            "## 1.2.3 - November  5, 2026",
        ),
        ("## {date:%d %b %Y}: {version}", "## 05 Nov 2026: 1.2.3"),
        ("## {version}-{date:%d.%m.%Y}", "## 1.2.3-05.11.2026"),
        (
            "## {version} ({date:100%% %m/%d/%Y})",
            "## 1.2.3 (100% 11/05/2026)",
        ),
    ] {
        let heading = ChangelogHeading::parse(format).unwrap();
        let changelog =
            insert_version_section(None, &heading, &prerelease, "2026-11-04", "Candidate.\n")
                .unwrap();
        let changelog = insert_version_section(
            Some(&changelog),
            &heading,
            &stable,
            "2026-11-05",
            "Stable.\n",
        )
        .unwrap();

        assert!(
            changelog.contains(&format!("\n{rendered}\n")),
            "{changelog}"
        );
        assert_eq!(
            read_version_section(&changelog, &heading, &stable).unwrap(),
            "Stable.\n",
            "{format}"
        );
        assert_eq!(
            read_version_section(&changelog, &heading, &prerelease).unwrap(),
            "Candidate.\n",
            "{format}"
        );
    }
}

#[test]
fn finds_only_valid_dates_in_the_declared_format() {
    let heading = ChangelogHeading::parse("## {version} ({date:%d.%m.%Y})").unwrap();
    let changelog =
        "# Changelog\n\n## 1.0.0 (29.02.2028)\n\nLeap.\n\n## 0.9.0 (29.02.2026)\n\nInvalid.\n";

    assert_eq!(
        read_version_section(changelog, &heading, &Version::new(1, 0, 0)).unwrap(),
        "Leap.\n"
    );
    assert!(matches!(
        read_version_section(changelog, &heading, &Version::new(0, 9, 0)).unwrap_err(),
        ChangelogError::MissingVersion(_)
    ));
}

#[test]
fn rejects_unsupported_date_formats() {
    for format in [
        "## {version} {date:%H:%M}",
        "## {version} {date:%d.%m.}",
        "## {version} {date:%Q}",
        "## {version} {date:%}",
        "## {version} {date:}",
        "## {version} {date:%Y",
        "## {version} {date} {date:%Y}",
        "## {version} {date:%Y{version}}",
    ] {
        assert!(
            matches!(
                ChangelogHeading::parse(format).unwrap_err(),
                ChangelogError::InvalidHeading(_)
            ),
            "{format}"
        );
    }
}
