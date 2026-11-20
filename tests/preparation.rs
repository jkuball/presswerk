mod support;

use anyhow::bail;
use git2::Oid;
use presswerk::analysis::GitCliffAnalyzer;
use presswerk::declaration::{
    Analyzer, BadgeOptions, BadgeVersionFile, CargoLockVersionFile, CargoWorkspaceVersionFile,
    Changelog, ConventionalAnalyzer, GitmojiAnalyzer, JsonVersionFile, Publishers,
    ReleaseDeclaration, Tag, TomlVersionFile, VersionFile,
};
use presswerk::evaluation::EvaluatedReleaseDeclaration;
use presswerk::preparation::{build_release_plan, is_worktree_dirty, prepare_release, render_plan};
use presswerk::publication::{
    PublicationError, Release, RemoteState, TagPublisher, VerifiedPredecessor,
    verify_predecessor_with,
};
use presswerk::publishers::Completeness;
use presswerk::publishers::github_release::{
    DEFAULT_GITHUB_RELEASE_BODY, GithubReleaseDeclaration,
};
use presswerk::publishers::oci::helm::OciHelmDeclaration;
use presswerk::publishers::oci::image::{Compression, OciImageDeclaration};
use presswerk::publishers::shared::secrets::Credential;
use semver::Version;
use std::cell::RefCell;
use std::collections::BTreeMap;
use support::{GitRepo, conventional_catalog, gitmoji_catalog};

fn declaration(version: &str) -> ReleaseDeclaration {
    let version = Version::parse(version).unwrap();
    ReleaseDeclaration {
        schema_version: 0,
        stable: version.major >= 1,
        version,
        version_files: vec![VersionFile::Toml(TomlVersionFile {
            path: "Cargo.toml".into(),
            toml: "package.version".into(),
        })],
        tag: Tag {
            format: "v{version}".into(),
        },
        analyzer: Analyzer::Gitmoji {
            gitmoji: GitmojiAnalyzer {
                catalog: gitmoji_catalog(),
            },
        },
        changelog: Changelog {
            path: "CHANGELOG.md".into(),
            template: presswerk::declaration::DEFAULT_CHANGELOG_TEMPLATE.into(),
            heading: presswerk::changelog::DEFAULT_HEADING.into(),
            unreleased_heading: presswerk::changelog::DEFAULT_UNRELEASED_HEADING.into(),
        },
        publishers: github_release("owner/project", BTreeMap::new()),
    }
}

fn github_release(repository: &str, assets: BTreeMap<String, String>) -> Publishers {
    let mut publishers = Publishers::new();
    publishers.insert(
        "github-release",
        "default",
        GithubReleaseDeclaration {
            api_url: "https://api.github.com".into(),
            repository: repository.into(),
            token: Credential::env("GITHUB_TOKEN"),
            body: DEFAULT_GITHUB_RELEASE_BODY.into(),
            assets,
        },
    );
    publishers
}

fn write_version(repository: &GitRepo, version: &str) {
    repository.write(
        "Cargo.toml",
        format!("[package]\nname = \"example\"\nversion = \"{version}\"\n"),
    );
}

fn evaluated(repository: &GitRepo, version: &str) -> EvaluatedReleaseDeclaration {
    EvaluatedReleaseDeclaration {
        commit: Oid::from_str(&repository.head()).unwrap(),
        declaration: declaration(version),
    }
}

fn predecessor(repository: &GitRepo, version: &str) -> VerifiedPredecessor {
    let tag = format!("v{version}");
    let reference = format!("{tag}^{{commit}}");
    VerifiedPredecessor {
        version: Version::parse(version).unwrap(),
        tag,
        commit: Oid::from_str(&repository.git(["rev-parse", &reference])).unwrap(),
        pending: false,
    }
}

struct FakeTagPublisher(RemoteState);

impl TagPublisher for FakeTagPublisher {
    fn state(&self, _release: &Release) -> Result<RemoteState, PublicationError> {
        Ok(self.0.clone())
    }

    fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
        unreachable!("predecessor verification never publishes")
    }
}

#[test]
fn plans_and_prepares_the_full_range_after_a_completed_release() {
    let repository = GitRepo::new();
    write_version(&repository, "0.1.0");
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## [0.1.0] - 2026-10-01\n\nFirst release.\n",
    );
    repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let feature = repository.commit("✨ Add full-range planning");
    repository.write("README.md", "documented\n");
    let latest = repository.commit("📝 Document the feature");
    let previous = predecessor(&repository, "0.1.0");
    let mut checks = Vec::new();

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |candidate, version, declaration| {
            checks.push((candidate, version.clone(), declaration.version.clone()));
            Ok(Some(previous.clone()))
        },
    )
    .unwrap();

    assert_eq!(
        checks,
        [(
            Oid::from_str(&latest).unwrap(),
            Version::new(0, 1, 0),
            Version::new(0, 1, 0)
        )]
    );
    assert_eq!(plan.analysis.commits.len(), 2);
    assert_eq!(plan.analysis.commits[0].id, latest);
    assert_eq!(plan.analysis.commits[1].id, feature);
    assert_eq!(plan.next_version, Some(Version::new(0, 2, 0)));
    assert!(render_plan(&plan).contains("Cargo.toml  0.1.0 -> 0.2.0"));

    let release = prepare_release(&plan).unwrap().unwrap();

    assert_eq!(repository.head(), release.to_string());
    assert_eq!(
        repository.git(["show", &format!("{release}:Cargo.toml")]),
        "[package]\nname = \"example\"\nversion = \"0.2.0\""
    );
    let changelog = repository.git(["show", &format!("{release}:CHANGELOG.md")]);
    assert!(changelog.contains("## [0.2.0] - 2026-11-05"));
    assert!(changelog.contains("\n- Add full-range planning"));
    assert!(changelog.contains("\n- Document the feature"));
}

#[test]
fn a_breaking_change_uses_the_declared_api_stability() {
    for (stable, expected) in [(false, "0.5.0"), (true, "1.0.0")] {
        let repository = GitRepo::new();
        write_version(&repository, "0.4.2");
        repository.write("breaking", "changed\n");
        repository.commit("💥 Change the public API");
        let mut evaluated = evaluated(&repository, "0.4.2");
        evaluated.declaration.stable = stable;

        let plan = build_release_plan(
            repository.path(),
            evaluated,
            &GitCliffAnalyzer,
            "2026-11-05",
            |_, _, _| Ok(None),
        )
        .unwrap();

        assert_eq!(plan.next_version, Some(Version::parse(expected).unwrap()));
        let release = prepare_release(&plan).unwrap().unwrap();
        assert_eq!(
            repository.git(["show", &format!("{release}:Cargo.toml")]),
            format!("[package]\nname = \"example\"\nversion = \"{expected}\"")
        );
    }
}

#[test]
fn prepares_the_declared_changelog_template() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.write("feature", "implemented\n");
    repository.commit("✨ (cli): Add search");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.changelog.template = "Version {{ version }}\n{% for group in groups %}{{ group.title }}\n{% for commit in group.commits %}{{ commit.text }}|{{ commit.message }}|{{ commit.scope }}|{{ commit.breaking }}|{{ commit.id }}\n{% endfor %}{% endfor %}".into();

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert!(plan.notes.starts_with(
        "Version 0.1.0\n✨ Introduce new features\n(cli): Add search|Add search|cli|False|"
    ));
    let release = prepare_release(&plan).unwrap().unwrap();
    let changelog = repository.git(["show", &format!("{release}:CHANGELOG.md")]);
    assert!(changelog.contains(plan.notes.trim_end()));
}

#[test]
fn moves_unreleased_text_into_the_prepared_section() {
    let repository = GitRepo::new();
    write_version(&repository, "0.1.0");
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## [0.1.0] - 2026-10-01\n\nFirst release.\n",
    );
    repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## [Unreleased]\n\nRead the migration notes.\n\n## [0.1.0] - 2026-10-01\n\nFirst release.\n",
    );
    repository.commit("📝 Write migration notes");
    let previous = predecessor(&repository, "0.1.0");

    // The hand-written text alone does not cause a release, but the plan shows it.
    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(Some(previous.clone())),
    )
    .unwrap();
    assert_eq!(plan.next_version, None);
    assert!(render_plan(&plan).contains("notes\n  Read the migration notes.\n"));

    repository.write("feature", "implemented\n");
    repository.commit("✨ Add migration");
    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(Some(previous.clone())),
    )
    .unwrap();
    assert!(
        plan.notes
            .starts_with("Read the migration notes.\n\n### ✨ Introduce new features\n")
    );

    let release = prepare_release(&plan).unwrap().unwrap();
    let changelog = repository.git(["show", &format!("{release}:CHANGELOG.md")]);
    assert!(changelog.starts_with(
        "# Changelog\n\n## [0.2.0] - 2026-11-05\n\nRead the migration notes.\n\n### ✨"
    ));
    assert!(changelog.ends_with("## [0.1.0] - 2026-10-01\n\nFirst release."));
}

#[test]
fn a_template_without_unreleased_leaves_the_unreleased_section_untouched() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## [Unreleased]\n\nRead the migration notes.\n",
    );
    repository.commit("✨ Add migration");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.changelog.template = "Release {{ version }}".into();

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    assert_eq!(plan.notes, "Release 0.1.0");

    let release = prepare_release(&plan).unwrap().unwrap();
    let changelog = repository.git(["show", &format!("{release}:CHANGELOG.md")]);
    assert!(changelog.starts_with("# Changelog\n\n## [0.1.0] - 2026-11-05\n\nRelease 0.1.0\n"));
    assert!(changelog.ends_with("## [Unreleased]\n\nRead the migration notes."));
}

#[test]
fn plans_the_first_release_from_the_implicit_zero_baseline() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("🎉 Begin project");
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add the first feature");

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.0.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert_eq!(plan.previous_release, None);
    assert_eq!(plan.analysis.commits.len(), 2);
    assert_eq!(plan.next_version, Some(Version::new(0, 1, 0)));
}

#[test]
fn replans_the_first_release_after_reverting_a_failed_candidate() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("🎉 Begin project");
    repository.write("feature", "implemented\n");
    let feature = repository.commit("✨ Add the first feature");
    write_version(&repository, "0.1.0");
    repository.write("CHANGELOG.md", "## [0.1.0]\n");
    let candidate = repository.commit("🔖 Release v0.1.0");
    repository.git(["revert", "--no-edit", &candidate]);
    repository.write("fix", "fixed\n");
    let fix = repository.commit("🐛 Fix the failed candidate");

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.0.0"),
        &GitCliffAnalyzer,
        "2026-11-06",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert_eq!(plan.previous_release, None);
    assert_eq!(plan.next_version, Some(Version::new(0, 1, 0)));
    assert_eq!(plan.analysis.commits.len(), 3);
    assert_eq!(plan.analysis.commits[0].id, fix);
    assert_eq!(plan.analysis.commits[1].id, feature);
    assert!(!plan.notes.contains("Revert"));
    assert!(!plan.notes.contains("⏪"));
}

#[test]
fn exits_without_a_commit_when_the_full_range_is_not_release_worthy() {
    let repository = GitRepo::new();
    write_version(&repository, "0.1.0");
    repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("README.md", "documented\n");
    let before = repository.commit("📝 Document the release");
    let previous = predecessor(&repository, "0.1.0");

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(Some(previous.clone())),
    )
    .unwrap();

    assert_eq!(plan.next_version, None);
    assert_eq!(prepare_release(&plan).unwrap(), None);
    assert_eq!(repository.head(), before);
}

#[test]
fn refuses_pending_and_incomplete_previous_releases() {
    let pending = GitRepo::new();
    write_version(&pending, "0.0.0");
    pending.commit("🎉 Begin project");
    write_version(&pending, "0.1.0");
    let release = pending.commit("🔖 Release v0.1.0");
    pending.write("README.md", "later\n");
    pending.commit("📝 Later change");

    let error = build_release_plan(
        pending.path(),
        evaluated(&pending, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| bail!("pending release 0.1.0 at {release} is not published yet"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("pending release 0.1.0"));
    assert!(error.to_string().contains(&release));

    let incomplete = GitRepo::new();
    write_version(&incomplete, "0.1.0");
    incomplete.commit("🔖 Release v0.1.0");
    incomplete.annotated_tag("v0.1.0");
    incomplete.write("feature", "implemented\n");
    incomplete.commit("✨ Add a feature");

    let error = build_release_plan(
        incomplete.path(),
        evaluated(&incomplete, "0.1.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| bail!("previous release v0.1.0: GitHub Release is a draft"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("previous release v0.1.0"));
    assert!(format!("{error:#}").contains("GitHub Release is a draft"));
}

#[test]
fn uses_the_exact_predecessor_declaration_for_release_state_checks() {
    let repository = GitRepo::new();
    write_version(&repository, "0.1.0");
    repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let previous = predecessor(&repository, "0.1.0");
    let mut current = evaluated(&repository, "0.1.0");
    current.declaration.publishers = github_release("new/project", BTreeMap::new());
    let mut previous_declaration = declaration("0.1.0");
    previous_declaration.publishers = github_release("old/project", BTreeMap::new());
    let evaluated_commits = RefCell::new(Vec::new());
    let checked_repositories = RefCell::new(Vec::new());
    let tag = FakeTagPublisher(RemoteState::Matching);

    let plan = build_release_plan(
        repository.path(),
        current,
        &GitCliffAnalyzer,
        "2026-11-05",
        |candidate, version, declaration| {
            verify_predecessor_with(
                repository.path(),
                candidate,
                version,
                declaration,
                &tag,
                None,
                |commit| {
                    evaluated_commits.borrow_mut().push(commit);
                    Ok(previous_declaration.clone())
                },
                |node, release, _| {
                    checked_repositories.borrow_mut().push(
                        node.publisher
                            .describe(&release.version, &release.tag)
                            .target,
                    );
                    Ok(Completeness::Complete)
                },
            )
            .map_err(Into::into)
        },
    )
    .unwrap();

    assert_eq!(plan.commit.to_string(), candidate);
    assert_eq!(plan.previous_release, Some(previous.clone()));
    assert_eq!(*evaluated_commits.borrow(), [previous.commit]);
    assert_eq!(*checked_repositories.borrow(), ["old/project"]);
}

#[test]
fn refuses_deleted_moved_and_lightweight_remote_predecessor_tags() {
    let repository = GitRepo::new();
    write_version(&repository, "0.1.0");
    repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add a feature");
    let previous_declaration = declaration("0.1.0");

    for (state, expected) in [
        (RemoteState::Missing, "missing from the remote"),
        (
            RemoteState::Conflicting("remote tag points to another commit".into()),
            "another commit",
        ),
        (
            RemoteState::Conflicting("remote tag is lightweight".into()),
            "lightweight",
        ),
    ] {
        let tag = FakeTagPublisher(state);
        let error = build_release_plan(
            repository.path(),
            evaluated(&repository, "0.1.0"),
            &GitCliffAnalyzer,
            "2026-11-05",
            |candidate, version, declaration| {
                verify_predecessor_with(
                    repository.path(),
                    candidate,
                    version,
                    declaration,
                    &tag,
                    None,
                    |_| Ok(previous_declaration.clone()),
                    |_, _, _| Ok(Completeness::Complete),
                )
                .map_err(Into::into)
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains(expected), "{error:#}");
    }
}

#[test]
fn prepares_a_release_from_a_detached_ci_checkout() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add the first feature");
    repository.git(["checkout", "--detach", "--quiet"]);

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.0.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    let release = prepare_release(&plan).unwrap().unwrap();

    assert_eq!(repository.head(), release.to_string());
    assert_eq!(
        repository.git(["show", &format!("{release}:Cargo.toml")]),
        "[package]\nname = \"example\"\nversion = \"0.1.0\""
    );
}

#[test]
fn plan_reads_committed_files_but_prepare_refuses_to_overwrite_a_release_file() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.write("feature", "committed\n");
    repository.commit("✨ Add the first feature");
    repository.write("Cargo.toml", "uncommitted and invalid\n");

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.0.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert_eq!(plan.current_version, Version::new(0, 0, 0));
    assert!(is_worktree_dirty(repository.path()).unwrap());
    let head = repository.head();
    let error = prepare_release(&plan).unwrap_err().to_string();
    assert!(
        error.contains("local changes in Cargo.toml would be overwritten"),
        "{error}"
    );
    assert_eq!(repository.head(), head);
}

#[test]
fn prepare_keeps_unrelated_local_changes_and_checks_out_the_release_files() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.write("feature", "committed\n");
    repository.commit("✨ Add the first feature");
    repository.write("feature", "uncommitted\n");
    repository.write("scratch", "untracked\n");

    let plan = build_release_plan(
        repository.path(),
        evaluated(&repository, "0.0.0"),
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    let release = prepare_release(&plan).unwrap().unwrap();

    assert_eq!(repository.head(), release.to_string());
    assert_eq!(
        repository.git(["show", &format!("{release}:feature")]),
        "committed"
    );
    assert_eq!(
        repository.git(["status", "--porcelain"]),
        "M feature\n?? scratch"
    );
    assert_eq!(
        std::fs::read_to_string(repository.path().join("Cargo.toml")).unwrap(),
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n"
    );
}

fn badge() -> VersionFile {
    VersionFile::Badge(BadgeVersionFile {
        path: ".github/presswerk-badge.svg".into(),
        badge: BadgeOptions::default(),
    })
}

#[test]
fn prepare_writes_several_keys_of_one_file() {
    let repository = GitRepo::new();
    write_version(&repository, "0.38.2");
    let lockfile = "{\n  \"version\": \"0.38.2\",\n  \"packages\": {\n    \"\": { \"version\": \"0.38.2\" }\n  }\n}\n";
    repository.write("package-lock.json", lockfile);
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.38.2");
    for key in ["version", r#"packages."".version"#] {
        evaluated
            .declaration
            .version_files
            .push(VersionFile::Json(JsonVersionFile {
                path: "package-lock.json".into(),
                json: key.into(),
            }));
    }

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    assert!(render_plan(&plan).contains("package-lock.json  0.38.2 -> 0.39.0"));

    let release = prepare_release(&plan).unwrap().unwrap();
    assert_eq!(
        repository.git(["show", &format!("{release}:package-lock.json")]),
        lockfile.replace("0.38.2", "0.39.0").trim_end()
    );
}

#[test]
fn prepare_generates_the_version_badge_without_reading_it() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.version_files.push(badge());

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    assert!(render_plan(&plan).contains(".github/presswerk-badge.svg  0.0.0 -> 0.1.0"));

    let release = prepare_release(&plan).unwrap().unwrap();
    let svg = repository.git(["show", &format!("{release}:.github/presswerk-badge.svg")]);
    assert!(svg.starts_with("<svg "));
    assert!(svg.contains("<title>Version 0.1.0, released with Presswerk</title>"));
}

#[test]
fn a_badge_alone_does_not_carry_the_version() {
    let repository = GitRepo::new();
    repository.write(".github/presswerk-badge.svg", "<svg>stale</svg>\n");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.version_files = vec![badge()];

    let error = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap_err();

    assert!(error.to_string().contains("carries the version"));
}

#[test]
fn the_release_commit_names_the_declared_tag() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.tag.format = "release-{version}".into();

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    assert_eq!(plan.next_tag.as_deref(), Some("release-0.1.0"));

    let release = prepare_release(&plan).unwrap().unwrap();
    assert_eq!(
        repository.git(["show", "--format=%s", "--no-patch", &release.to_string()]),
        "🔖 Release release-0.1.0"
    );
}

#[test]
fn the_plan_of_a_release_without_publishers_says_so() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated
        .declaration
        .publishers
        .remove_type("github-release");

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert!(render_plan(&plan).contains("\npublishers\n  none\n\nnotes\n"));
}

#[test]
fn the_plan_lists_each_publisher_with_its_release_and_assets() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.publishers = github_release(
        "owner/project",
        BTreeMap::from([(
            "example-x86_64-linux".into(),
            "/nix/store/abc-example-0.1.0/bin/example".into(),
        )]),
    );

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert!(render_plan(&plan).contains(concat!(
        "\npublishers\n",
        "  github-release.default  owner/project\n",
        "    release  v0.1.0\n",
        "    asset    example-x86_64-linux\n",
    )));
}

fn oci_image_publisher(tags: &[&str], aliases: &[&str]) -> OciImageDeclaration {
    OciImageDeclaration {
        repository: "ghcr.io/owner/project".into(),
        platforms: BTreeMap::from([
            ("linux/amd64".into(), "/nix/store/abc-image.tar.gz".into()),
            ("linux/arm64".into(), "/nix/store/def-image.tar.gz".into()),
        ]),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        public: false,
        compression: Compression::default(),
        username: Credential::env("REGISTRY_USERNAME"),
        password: Credential::env("REGISTRY_TOKEN"),
    }
}

#[test]
fn the_plan_lists_oci_images_and_rendered_tags_before_the_release() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.publishers.insert(
        "oci-image",
        "default",
        oci_image_publisher(&["{version}"], &["latest", "{major}.{minor}"]),
    );

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert!(render_plan(&plan).contains(concat!(
        "\npublishers\n",
        "  oci-image.default  ghcr.io/owner/project\n",
        "    image    linux/amd64\n",
        "    image    linux/arm64\n",
        "    tag      0.1.0\n",
        "    alias    latest\n",
        "    alias    0.1\n",
        "  github-release.default  owner/project\n",
        "    release  v0.1.0\n",
    )));
}

#[test]
fn the_plan_lists_an_oci_helm_chart_under_its_release_tag() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.publishers.insert(
        "oci-helm",
        "default",
        OciHelmDeclaration {
            repository: "ghcr.io/owner/charts/project".into(),
            chart: "/nix/store/abc-project-0.1.0.tgz".into(),
            public: true,
            username: Credential::env("REGISTRY_USERNAME"),
            password: Credential::env("REGISTRY_TOKEN"),
        },
    );

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert!(render_plan(&plan).contains(concat!(
        "  oci-helm.default  ghcr.io/owner/charts/project\n",
        "    tag      ghcr.io/owner/charts/project:0.1.0\n",
        "  github-release.default  owner/project\n",
    )));
}

#[test]
fn the_plan_rejects_an_oci_helm_tag_shared_with_an_oci_image_publisher() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.0.0");
    // Only the next version collides: the alias names it, and the chart's tag is the version.
    // Each type's Nix module derives the claims into the declaration, as tag templates.
    let claimed = |declaration: serde_json::Value, keys: &[&str]| {
        let mut declaration = declaration;
        declaration["claims"] = keys
            .iter()
            .map(|key| serde_json::json!({ "destination": "oci://ghcr.io/owner/project", "key": key }))
            .collect();
        declaration
    };
    evaluated.declaration.publishers.insert(
        "oci-image",
        "image",
        claimed(
            serde_json::to_value(oci_image_publisher(&["v{version}"], &["0.1.0"])).unwrap(),
            &["v{version}", "0.1.0"],
        ),
    );
    evaluated.declaration.publishers.insert(
        "oci-helm",
        "chart",
        claimed(
            serde_json::to_value(OciHelmDeclaration {
                repository: "ghcr.io/owner/project".into(),
                chart: "/nix/store/abc-project-0.1.0.tgz".into(),
                public: true,
                username: Credential::env("REGISTRY_USERNAME"),
                password: Credential::env("REGISTRY_TOKEN"),
            })
            .unwrap(),
            &["{version}"],
        ),
    );

    let error = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("oci-image.image and oci-helm.chart"),
        "{error}"
    );
    assert!(error.contains("both render 0.1.0"), "{error}");
}

#[test]
fn conventional_commits_plan_notes_and_a_conventional_release_commit() {
    let repository = GitRepo::new();
    write_version(&repository, "0.0.0");
    repository.commit("chore: Begin the project");
    repository.write("feature", "implemented\n");
    repository.commit("feat(cli): Add --json");
    repository.write("fix", "fixed\n");
    repository.commit("fix: Keep the tag order stable");
    repository.write("unrelated", "changed\n");
    repository.commit("Update the readme");
    let mut evaluated = evaluated(&repository, "0.0.0");
    evaluated.declaration.analyzer = Analyzer::Conventional {
        conventional: ConventionalAnalyzer {
            catalog: conventional_catalog(),
        },
    };

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-12",
        |_, _, _| Ok(None),
    )
    .unwrap();

    assert_eq!(plan.next_version, Some(Version::new(0, 1, 0)));
    assert_eq!(plan.analysis.commits.len(), 3);
    assert_eq!(
        plan.notes,
        "### Features\n\n- **cli:** Add --json\n\n### Bug fixes\n\n- Keep the tag order stable\n\n### Chores\n\n- Begin the project\n"
    );
    assert_eq!(
        plan.release_subject.as_deref(),
        Some("chore(release): v0.1.0")
    );

    let release = prepare_release(&plan).unwrap().unwrap();
    assert_eq!(
        repository.git(["show", "--format=%s", "--no-patch", &release.to_string()]),
        "chore(release): v0.1.0"
    );
}

#[test]
fn prepares_a_cargo_workspace_release() {
    let repository = GitRepo::new();
    repository.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
    );
    repository.write(
        "crates/a/Cargo.toml",
        "[package]\nname = \"a\"\nversion.workspace = true\n",
    );
    repository.write(
        "crates/b/Cargo.toml",
        "[package]\nname = \"b\"\nversion = \"3.0.0\"\n",
    );
    let lock = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"b\"\nversion = \"3.0.0\"\n";
    repository.write("Cargo.lock", lock);
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.1.0");
    evaluated.declaration.version_files = vec![
        VersionFile::Toml(TomlVersionFile {
            path: "Cargo.toml".into(),
            toml: "workspace.package.version".into(),
        }),
        VersionFile::CargoWorkspace(CargoWorkspaceVersionFile {
            path: "Cargo.lock".into(),
            cargo_workspace: "Cargo.toml".into(),
        }),
    ];

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    assert!(render_plan(&plan).contains("Cargo.lock  0.1.0 -> 0.2.0"));

    let release = prepare_release(&plan).unwrap().unwrap();
    assert_eq!(
        repository.git(["show", &format!("{release}:Cargo.lock")]),
        lock.replacen("0.1.0", "0.2.0", 1).trim_end()
    );
    assert!(
        repository
            .git(["show", &format!("{release}:Cargo.toml")])
            .contains("version = \"0.2.0\"")
    );
}

#[test]
fn prepares_a_cargo_workspace_and_an_own_version_member_in_one_lock_file() {
    let repository = GitRepo::new();
    repository.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
    );
    repository.write(
        "a/Cargo.toml",
        "[package]\nname = \"a\"\nversion.workspace = true\n",
    );
    repository.write(
        "b/Cargo.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
    );
    let lock = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"b\"\nversion = \"0.1.0\"\n";
    repository.write("Cargo.lock", lock);
    repository.commit("✨ Add the first feature");
    let mut evaluated = evaluated(&repository, "0.1.0");
    evaluated.declaration.version_files = vec![
        VersionFile::Toml(TomlVersionFile {
            path: "Cargo.toml".into(),
            toml: "workspace.package.version".into(),
        }),
        VersionFile::CargoWorkspace(CargoWorkspaceVersionFile {
            path: "Cargo.lock".into(),
            cargo_workspace: "Cargo.toml".into(),
        }),
        VersionFile::Toml(TomlVersionFile {
            path: "b/Cargo.toml".into(),
            toml: "package.version".into(),
        }),
        VersionFile::CargoLock(CargoLockVersionFile {
            path: "Cargo.lock".into(),
            cargo_lock: "b".into(),
        }),
    ];

    let plan = build_release_plan(
        repository.path(),
        evaluated,
        &GitCliffAnalyzer,
        "2026-11-05",
        |_, _, _| Ok(None),
    )
    .unwrap();
    let release = prepare_release(&plan).unwrap().unwrap();

    assert_eq!(
        repository.git(["show", &format!("{release}:Cargo.lock")]),
        lock.replace("0.1.0", "0.2.0").trim_end()
    );
    assert!(
        repository
            .git(["show", &format!("{release}:b/Cargo.toml")])
            .contains("version = \"0.2.0\"")
    );
}
