mod support;

use presswerk::analysis::{
    Analysis, AnalysisError, Analyzer, Bump, CommitRange, GitCliffAnalyzer, render_release_notes,
};
use presswerk::declaration::{
    self, ConventionalAnalyzer, ConventionalCatalog, GitmojiAnalyzer, GitmojiCatalog, Tag,
};
use support::{GitRepo, conventional_catalog, gitmoji_catalog};

fn gitmoji(catalog: GitmojiCatalog) -> declaration::Analyzer {
    declaration::Analyzer::Gitmoji {
        gitmoji: GitmojiAnalyzer { catalog },
    }
}

fn conventional(catalog: ConventionalCatalog) -> declaration::Analyzer {
    declaration::Analyzer::Conventional {
        conventional: ConventionalAnalyzer { catalog },
    }
}

fn tag() -> Tag {
    Tag {
        format: "v{version}".into(),
    }
}

fn commit(repository: &GitRepo, sequence: usize, subject: &str) -> String {
    repository.write("history", format!("{sequence}\n"));
    repository.commit(subject)
}

struct RejectingAnalyzer;

impl Analyzer for RejectingAnalyzer {
    fn analyze(
        &self,
        _range: &CommitRange,
        _analyzer: &declaration::Analyzer,
        _tag: &Tag,
    ) -> Result<Analysis, AnalysisError> {
        Err(AnalysisError::new("external analyzer failure"))
    }
}

#[test]
fn analyzes_initial_history_and_omits_non_entries() {
    let repository = GitRepo::new();
    commit(&repository, 0, "🔖 Previous release");
    let feature = commit(&repository, 1, "✨ Add release planning");
    let documentation = commit(&repository, 2, ":memo: Document release planning");
    commit(&repository, 3, "Merge pull request without Gitmoji");
    let fix = commit(&repository, 4, ":bug: Fix release planning");
    let range = CommitRange::initial(repository.path(), repository.head()).unwrap();

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    assert_eq!(analysis.commits.len(), 4);
    assert!(analysis.commits.iter().any(|commit| commit.id == feature));
    assert!(
        analysis
            .commits
            .iter()
            .any(|commit| commit.id == documentation && commit.bump().is_none())
    );
    assert!(analysis.commits.iter().any(|commit| commit.id == fix));
    assert!(
        analysis
            .commits
            .iter()
            .any(|commit| commit.intention.gitmoji().unwrap().shortcode == ":bookmark:")
    );
}

#[test]
fn bookmark_uses_the_declared_release_level() {
    let repository = GitRepo::new();
    let bookmark = commit(&repository, 0, "🔖 Mark a milestone");
    let range = CommitRange::initial(repository.path(), "HEAD").unwrap();
    let mut catalog = gitmoji_catalog();
    catalog.get_mut("bookmark").unwrap().semver = Some(Bump::Patch);

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(catalog), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Patch));
    assert_eq!(analysis.commits.len(), 1);
    assert_eq!(analysis.commits[0].id, bookmark);
}

#[test]
fn initial_range_includes_the_root_and_pins_head() {
    let repository = GitRepo::new();
    let root = commit(&repository, 0, "✨ Root feature");
    let range = CommitRange::initial(repository.path(), "HEAD").unwrap();
    commit(&repository, 1, "💥 Later breaking change");

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    assert_eq!(analysis.commits.len(), 1);
    assert_eq!(analysis.commits[0].id, root);
}

#[test]
fn between_range_honors_the_base_and_pins_head() {
    let repository = GitRepo::new();
    commit(&repository, 0, "💥 Old breaking change");
    let base = commit(&repository, 1, "🔖 Previous release");
    let feature = commit(&repository, 2, "✨ Add range analysis");
    let range = CommitRange::between(repository.path(), base, "HEAD").unwrap();
    commit(&repository, 3, "💥 Later breaking change");

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    assert_eq!(analysis.commits.len(), 1);
    assert_eq!(analysis.commits[0].id, feature);
}

#[test]
fn repository_analysis_rejects_leading_whitespace() {
    let repository = GitRepo::new();
    commit(&repository, 0, "Begin project without Gitmoji");
    repository.write("history", "1\n");
    repository.git(["add", "--all"]);
    repository.git([
        "commit",
        "--quiet",
        "--cleanup=verbatim",
        "--message",
        "  ✨ Leading whitespace feature",
    ]);
    let leading = repository.head();
    let raw_commit = repository.git_output(["cat-file", "commit", &leading]);
    assert!(raw_commit.status.success());
    assert!(
        String::from_utf8(raw_commit.stdout)
            .unwrap()
            .ends_with("\n\n  ✨ Leading whitespace feature\n")
    );
    let range = CommitRange::initial(repository.path(), "HEAD").unwrap();

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap();

    assert_eq!(analysis.bump, None);
    assert!(analysis.commits.is_empty());
}

#[test]
fn external_analyzers_can_construct_errors() {
    let repository = GitRepo::new();
    let target = commit(&repository, 0, "Begin project without Gitmoji");
    let range = CommitRange::initial(repository.path(), target.clone()).unwrap();
    assert_eq!(range.repository(), repository.path());
    assert_eq!(range.base(), None);
    assert_eq!(range.target(), target);

    let error = RejectingAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap_err();

    assert_eq!(error.to_string(), "external analyzer failure");
}

fn revert(repository: &GitRepo, arguments: &[&str]) -> String {
    repository.git(
        ["revert", "--no-edit"]
            .into_iter()
            .chain(arguments.iter().copied()),
    );
    repository.head()
}

fn merge(repository: &GitRepo, branches: &[&str], subject: &str) -> String {
    repository.git(
        ["merge", "--quiet", "--no-ff", "--message", subject]
            .into_iter()
            .chain(branches.iter().copied()),
    );
    repository.head()
}

fn commit_file(repository: &GitRepo, path: &str, subject: &str) -> String {
    repository.write(path, format!("{subject}\n"));
    repository.commit(subject)
}

fn analyze(repository: &GitRepo, base: Option<&str>) -> Result<Analysis, AnalysisError> {
    let range = match base {
        Some(base) => CommitRange::between(repository.path(), base, "HEAD"),
        None => CommitRange::initial(repository.path(), "HEAD"),
    }
    .unwrap();
    GitCliffAnalyzer.analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
}

fn ids(analysis: &Analysis) -> Vec<&str> {
    analysis
        .commits
        .iter()
        .map(|commit| commit.id.as_str())
        .collect()
}

#[test]
fn exclusion_skips_only_the_candidate_oid() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let bookmark = commit_file(&repository, "bookmark", "🔖 Mark an ordinary milestone");
    let candidate = commit_file(&repository, "candidate", "🔖 Prepare release candidate");
    let range = CommitRange::between(repository.path(), root, &candidate)
        .unwrap()
        .excluding(candidate.parse().unwrap());
    let mut catalog = gitmoji_catalog();
    catalog.get_mut("bookmark").unwrap().semver = Some(Bump::Patch);

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(catalog), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Patch));
    assert_eq!(ids(&analysis), [bookmark]);
}

#[test]
fn excluding_a_merge_candidate_keeps_secondary_parent_commits() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let main = repository.git(["branch", "--show-current"]);
    repository.git(["checkout", "--quiet", "-b", "feature"]);
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    repository.git(["checkout", "--quiet", &main]);
    let fix = commit_file(&repository, "fix", "🐛 Fix main");
    let merged = merge(
        &repository,
        &["feature"],
        "💥 Merge feature with a Gitmoji subject",
    );
    assert_eq!(
        repository
            .git(["rev-list", "--parents", "-n1", &merged])
            .split(' ')
            .count(),
        3
    );

    let range = CommitRange::between(repository.path(), root, &merged)
        .unwrap()
        .excluding(merged.parse().unwrap());
    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(gitmoji_catalog()), &tag())
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    let mut analyzed = ids(&analysis);
    analyzed.sort_unstable();
    let mut expected = vec![feature.as_str(), fix.as_str()];
    expected.sort_unstable();
    assert_eq!(analyzed, expected);
}

#[test]
fn reverts_inside_the_range_cancel_their_targets() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    let documentation = commit_file(&repository, "docs", "📝 Document project");
    revert(&repository, &[&feature]);

    let analysis = analyze(&repository, Some(&root)).unwrap();

    assert_eq!(ids(&analysis), [documentation.as_str()]);
    assert_eq!(analysis.bump, None);
}

#[test]
fn revert_identity_comes_from_the_body_not_the_subject() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let first = commit_file(&repository, "first", "✨ Add first feature");
    let second = commit_file(&repository, "second", "✨ Add second feature");
    repository.write("first", "removed\n");
    repository.commit(&format!(
        "🐛 Undo the first feature\n\nThis reverts commit {}.",
        &first[..7]
    ));
    repository.write("second", "removed\n");
    let fake = repository.commit(&format!(
        "Revert \"✨ Add second feature\"\n\nNo trailer for {second}."
    ));

    let analysis = analyze(&repository, Some(&root)).unwrap();

    assert_eq!(ids(&analysis), [second.as_str()]);
    assert!(!ids(&analysis).contains(&fake.as_str()));
}

#[test]
fn reverts_of_released_commits_become_revert_entries() {
    let repository = GitRepo::new();
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    let release = commit_file(&repository, "release", "🔖 Release v0.1.0");
    let reverted = revert(&repository, &[&feature]);

    let analysis = analyze(&repository, Some(&release)).unwrap();

    assert_eq!(analysis.bump, Some(Bump::Patch));
    assert_eq!(analysis.commits.len(), 1);
    let entry = &analysis.commits[0];
    assert_eq!(entry.id, reverted);
    assert_eq!(entry.subject, "⏪️ Revert \"✨ Add feature\"");
    assert_eq!(entry.message, "Revert \"✨ Add feature\"");
    assert_eq!(entry.intention.gitmoji().unwrap().emoji, "⏪️");
    assert_eq!(entry.intention.gitmoji().unwrap().shortcode, ":rewind:");
    assert_eq!(entry.scope, None);
}

#[test]
fn a_released_revert_does_not_require_a_rewind_catalog_entry() {
    let repository = GitRepo::new();
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    let release = commit_file(&repository, "release", "🔖 Release v0.1.0");
    revert(&repository, &[&feature]);
    let range = CommitRange::between(repository.path(), release, "HEAD").unwrap();
    let mut catalog = gitmoji_catalog();
    catalog.remove("rewind");

    let analysis = GitCliffAnalyzer
        .analyze(&range, &gitmoji(catalog), &tag())
        .unwrap();

    assert_eq!(analysis.bump, None);
    assert!(analysis.commits.is_empty());
}

#[test]
fn reverts_of_reverts_alternate_by_parity() {
    for depth in 1..=4 {
        let repository = GitRepo::new();
        let root = commit_file(&repository, "root", "🎉 Begin project");
        let feature = commit_file(&repository, "feature", "✨ Add feature");
        let mut latest = feature.clone();
        for _ in 0..depth {
            latest = revert(&repository, &[&latest]);
        }

        let analysis = analyze(&repository, Some(&root)).unwrap();

        if depth % 2 == 0 {
            assert_eq!(ids(&analysis), [feature.as_str()], "depth {depth}");
        } else {
            assert!(analysis.commits.is_empty(), "depth {depth}");
        }
    }
}

#[test]
fn reverted_merges_suppress_only_their_branch_commits_in_range() {
    let repository = GitRepo::new();
    let main = repository.git(["branch", "--show-current"]);
    commit_file(&repository, "root", "🎉 Begin project");
    repository.git(["checkout", "--quiet", "-b", "feature"]);
    commit_file(&repository, "early", "✨ Add early branch work");
    repository.git(["checkout", "--quiet", &main]);
    let base = commit_file(&repository, "base", "🔖 Release v0.1.0");
    repository.git(["checkout", "--quiet", "feature"]);
    commit_file(&repository, "first", "✨ Add first branch feature");
    commit_file(&repository, "second", "🐛 Fix branch feature");
    repository.git(["checkout", "--quiet", &main]);
    let mainline = commit_file(&repository, "main", "📝 Document main");
    let merged = merge(&repository, &["feature"], "🔀 Merge feature");
    revert(&repository, &["-m", "1", &merged]);

    let analysis = analyze(&repository, Some(&base)).unwrap();

    assert_eq!(ids(&analysis), [mainline.as_str()]);
}

#[test]
fn reverted_octopus_merges_suppress_every_secondary_branch() {
    let repository = GitRepo::new();
    let main = repository.git(["branch", "--show-current"]);
    let root = commit_file(&repository, "root", "🎉 Begin project");
    for branch in ["left", "right"] {
        repository.git(["checkout", "--quiet", "-b", branch, &root]);
        commit_file(&repository, branch, &format!("✨ Add {branch} feature"));
    }
    repository.git(["checkout", "--quiet", &main]);
    let mainline = commit_file(&repository, "main", "📝 Document main");
    let merged = merge(&repository, &["left", "right"], "🔀 Merge both features");
    assert_eq!(
        repository
            .git(["rev-list", "--parents", "-n1", &merged])
            .split(' ')
            .count(),
        4
    );

    let before = analyze(&repository, Some(&root)).unwrap();
    assert_eq!(before.commits.len(), 3);

    revert(&repository, &["-m", "1", &merged]);
    let analysis = analyze(&repository, Some(&root)).unwrap();

    assert_eq!(ids(&analysis), [mainline.as_str()]);
}

#[test]
fn active_reverts_with_invalid_targets_fail() {
    for (body, expected) in [
        (
            "This reverts commit 0123456789abcdef0123456789abcdef01234567.".to_owned(),
            "does not resolve to exactly one commit",
        ),
        (
            "This reverts commit not-a-commit.".to_owned(),
            "malformed commit ID",
        ),
        ("This reverts commit abc.".to_owned(), "malformed commit ID"),
    ] {
        let repository = GitRepo::new();
        let root = commit_file(&repository, "root", "🎉 Begin project");
        repository.write("broken", "revert\n");
        repository.commit(&format!("Revert \"something\"\n\n{body}"));

        let error = analyze(&repository, Some(&root)).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn active_reverts_with_ambiguous_targets_fail() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    let prefix = &feature[..4];
    let git = git2::Repository::open(repository.path()).unwrap();
    let colliding = (0_u64..)
        .map(|seed| seed.to_string())
        .find(|contents| {
            git2::Oid::hash_object(git2::ObjectType::Blob, contents.as_bytes())
                .unwrap()
                .to_string()
                .starts_with(prefix)
        })
        .unwrap();
    git.blob(colliding.as_bytes()).unwrap();
    repository.write("feature", "removed\n");
    repository.commit(&format!(
        "Revert \"✨ Add feature\"\n\nThis reverts commit {prefix}."
    ));

    let error = analyze(&repository, Some(&root)).unwrap_err();

    assert!(error.to_string().contains("ambiguous"), "{error}");
}

#[test]
fn invalid_reverts_reverted_by_newer_reverts_are_ignored() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    repository.write("broken", "revert\n");
    let broken = repository.commit(
        "Revert \"something\"\n\nThis reverts commit 0123456789abcdef0123456789abcdef01234567.",
    );
    revert(&repository, &[&broken]);
    let feature = commit_file(&repository, "feature", "✨ Add feature");

    let analysis = analyze(&repository, Some(&root)).unwrap();

    assert_eq!(ids(&analysis), [feature.as_str()]);
}

#[test]
fn reverting_merges_against_other_mainlines_fails() {
    let repository = GitRepo::new();
    let main = repository.git(["branch", "--show-current"]);
    let root = commit_file(&repository, "root", "🎉 Begin project");
    repository.git(["checkout", "--quiet", "-b", "feature"]);
    commit_file(&repository, "feature", "✨ Add feature");
    repository.git(["checkout", "--quiet", &main]);
    commit_file(&repository, "main", "📝 Document main");
    let merged = merge(&repository, &["feature"], "🔀 Merge feature");
    revert(&repository, &["-m", "2", &merged]);

    let error = analyze(&repository, Some(&root)).unwrap_err();

    assert!(
        error.to_string().contains("not its first parent"),
        "{error}"
    );
}

#[test]
fn gitmoji_reverts_of_released_commits_become_revert_entries() {
    let repository = GitRepo::new();
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    let release = commit_file(&repository, "release", "🔖 Release v0.1.0");
    repository.write("feature", "removed\n");
    let reverted = repository.commit(&format!(
        "💥 Remove the feature again\n\nThis reverts commit {feature}."
    ));

    let analysis = analyze(&repository, Some(&release)).unwrap();

    assert_eq!(analysis.bump, Some(Bump::Patch));
    assert_eq!(analysis.commits.len(), 1);
    let entry = &analysis.commits[0];
    assert_eq!(entry.id, reverted);
    assert_eq!(entry.subject, "⏪️ 💥 Remove the feature again");
    assert_eq!(entry.message, "💥 Remove the feature again");
    assert_eq!(entry.intention.gitmoji().unwrap().shortcode, ":rewind:");
    assert_eq!(
        render_release_notes(&analysis).unwrap(),
        "### ⏪️ Revert changes\n\n- 💥 Remove the feature again\n"
    );
}

#[test]
fn only_the_first_body_line_identifies_a_revert() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "🎉 Begin project");
    let feature = commit_file(&repository, "feature", "✨ Add feature");
    repository.write("squashed", "squashed\n");
    let squashed = repository.commit(&format!(
        "🐛 Fix several things (#1)\n\n* 🐛 Fix one thing\n\n* Revert \"✨ Add feature\"\n\nThis reverts commit {feature}."
    ));

    let analysis = analyze(&repository, Some(&root)).unwrap();

    assert_eq!(ids(&analysis), [squashed.as_str(), feature.as_str()]);
}

#[test]
fn released_merges_reverted_against_any_mainline_become_revert_entries() {
    let repository = GitRepo::new();
    let main = repository.git(["branch", "--show-current"]);
    commit_file(&repository, "root", "🎉 Begin project");
    repository.git(["checkout", "--quiet", "-b", "feature"]);
    commit_file(&repository, "feature", "✨ Add feature");
    repository.git(["checkout", "--quiet", &main]);
    commit_file(&repository, "main", "📝 Document main");
    let merged = merge(&repository, &["feature"], "🔀 Merge feature");
    let release = commit_file(&repository, "release", "🔖 Release v0.1.0");
    let reverted = revert(&repository, &["-m", "2", &merged]);

    let analysis = analyze(&repository, Some(&release)).unwrap();

    assert_eq!(ids(&analysis), [reverted.as_str()]);
    assert_eq!(
        analysis.commits[0].intention.gitmoji().unwrap().shortcode,
        ":rewind:"
    );
    assert_eq!(
        analysis.commits[0].subject,
        "⏪️ Revert \"🔀 Merge feature\""
    );
}

#[test]
fn reverting_merges_without_a_valid_mainline_fails() {
    for (mainline, expected) in [
        (None, "without recording its mainline parent"),
        (Some("not-a-commit"), "malformed commit ID"),
        (
            Some("0123456789abcdef0123456789abcdef01234567"),
            "does not resolve to exactly one commit",
        ),
    ] {
        let repository = GitRepo::new();
        let main = repository.git(["branch", "--show-current"]);
        let root = commit_file(&repository, "root", "🎉 Begin project");
        repository.git(["checkout", "--quiet", "-b", "feature"]);
        commit_file(&repository, "feature", "✨ Add feature");
        repository.git(["checkout", "--quiet", &main]);
        commit_file(&repository, "main", "📝 Document main");
        let merged = merge(&repository, &["feature"], "🔀 Merge feature");
        repository.write("feature", "removed\n");
        let body = match mainline {
            Some(mainline) => {
                format!("This reverts commit {merged}, reversing\nchanges made to {mainline}.")
            }
            None => format!("This reverts commit {merged}."),
        };
        repository.commit(&format!("Revert \"🔀 Merge feature\"\n\n{body}"));

        let error = analyze(&repository, Some(&root)).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

fn analyze_conventional(
    repository: &GitRepo,
    base: Option<&str>,
    catalog: ConventionalCatalog,
) -> Analysis {
    let range = match base {
        Some(base) => CommitRange::between(repository.path(), base, "HEAD"),
        None => CommitRange::initial(repository.path(), "HEAD"),
    }
    .unwrap();
    GitCliffAnalyzer
        .analyze(&range, &conventional(catalog), &tag())
        .unwrap()
}

#[test]
fn conventional_analysis_omits_other_subjects_and_merges() {
    let repository = GitRepo::new();
    let main = repository.git(["branch", "--show-current"]);
    let root = commit_file(&repository, "root", "chore: Begin project");
    repository.git(["checkout", "--quiet", "-b", "feature"]);
    let feature = commit_file(&repository, "feature", "feat(cli): Add --json");
    repository.git(["checkout", "--quiet", &main]);
    let docs = commit_file(&repository, "docs", "docs: Document --json");
    commit_file(&repository, "plain", "Update the readme");
    commit_file(&repository, "unknown", "deps: Bump serde");
    commit_file(&repository, "gitmoji", "✨ Add a Gitmoji feature");
    merge(&repository, &["feature"], "feat: Merge the feature branch");

    let analysis = analyze_conventional(&repository, Some(&root), conventional_catalog());

    assert_eq!(analysis.bump, Some(Bump::Minor));
    let mut analyzed = ids(&analysis);
    analyzed.sort_unstable();
    let mut expected = vec![feature.as_str(), docs.as_str()];
    expected.sort_unstable();
    assert_eq!(analyzed, expected);
    let feature = analysis
        .commits
        .iter()
        .find(|commit| commit.id == feature)
        .unwrap();
    assert_eq!(feature.intention.conventional().unwrap().name, "feat");
    assert_eq!(feature.scope.as_deref(), Some("cli"));
    assert_eq!(feature.message, "Add --json");
}

#[test]
fn conventional_breaking_footers_in_commit_bodies_select_major() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "chore: Begin project");
    repository.write("fix", "fixed\n");
    repository.commit("fix: Rename the output field\n\nBREAKING CHANGE: `version` is now `next`.");

    let analysis = analyze_conventional(&repository, Some(&root), conventional_catalog());

    assert_eq!(analysis.bump, Some(Bump::Major));
    assert!(analysis.commits[0].breaking);
    assert_eq!(
        analysis
            .next_version(Some(&semver::Version::new(0, 3, 1)))
            .unwrap(),
        Some(semver::Version::new(0, 4, 0))
    );
    assert_eq!(
        render_release_notes(&analysis).unwrap(),
        "### Bug fixes\n\n- **BREAKING** Rename the output field\n"
    );
}

#[test]
fn conventional_release_commits_are_not_changes() {
    let repository = GitRepo::new();
    let root = commit_file(&repository, "root", "chore: Begin project");
    commit_file(&repository, "release", "chore(release): v0.1.0");
    let tooling = commit_file(
        &repository,
        "tooling",
        "chore(release): Update the workflow",
    );
    let other_format = commit_file(&repository, "other", "chore(release): release-0.2.0");
    let candidate = commit_file(&repository, "candidate", "chore(release): v0.2.0");
    let mut catalog = conventional_catalog();
    catalog.get_mut("chore").unwrap().semver = Some(Bump::Patch);

    let range = CommitRange::between(repository.path(), root, &candidate).unwrap();
    let analysis = GitCliffAnalyzer
        .analyze(&range, &conventional(catalog), &tag())
        .unwrap();

    let mut analyzed = ids(&analysis);
    analyzed.sort_unstable();
    let mut expected = vec![tooling.as_str(), other_format.as_str()];
    expected.sort_unstable();
    assert_eq!(analyzed, expected);
}

#[test]
fn conventional_reverts_of_released_commits_use_the_revert_type() {
    let repository = GitRepo::new();
    let feature = commit_file(&repository, "feature", "feat: Add feature");
    let fix = commit_file(&repository, "fix", "fix: Fix feature");
    let release = commit_file(&repository, "release", "chore(release): v0.1.0");
    let reverted = revert(&repository, &[&feature]);
    repository.write("fix", "undone\n");
    let typed = repository.commit(&format!(
        "fix(cli): Restore the old output\n\nThis reverts commit {fix}."
    ));

    let analysis = analyze_conventional(&repository, Some(&release), conventional_catalog());

    assert_eq!(analysis.bump, Some(Bump::Patch));
    assert_eq!(ids(&analysis), [typed.as_str(), reverted.as_str()]);
    let entry = &analysis.commits[1];
    assert_eq!(entry.intention.conventional().unwrap().name, "revert");
    assert_eq!(entry.message, "Revert \"feat: Add feature\"");
    assert_eq!(
        render_release_notes(&analysis).unwrap(),
        "### Bug fixes\n\n- **cli:** Restore the old output\n\n### Reverts\n\n- Revert \"feat: Add feature\"\n"
    );

    let mut catalog = conventional_catalog();
    catalog.remove("revert");
    let analysis = analyze_conventional(&repository, Some(&release), catalog);
    assert_eq!(ids(&analysis), [typed.as_str()]);
}
