mod support;

use presswerk::analysis::{Analysis, AnalysisError, Analyzer, Bump, CommitRange, GitCliffAnalyzer};
use support::GitRepo;

fn commit(repository: &GitRepo, sequence: usize, subject: &str) -> String {
    repository.write("history", format!("{sequence}\n"));
    repository.commit(subject)
}

struct RejectingAnalyzer;

impl Analyzer for RejectingAnalyzer {
    fn analyze(&self, _range: &CommitRange) -> Result<Analysis, AnalysisError> {
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

    let analysis = GitCliffAnalyzer.analyze(&range).unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    assert_eq!(analysis.commits.len(), 3);
    assert!(analysis.commits.iter().any(|commit| commit.id == feature));
    assert!(
        analysis
            .commits
            .iter()
            .any(|commit| commit.id == documentation && commit.gitmoji.bump.is_none())
    );
    assert!(analysis.commits.iter().any(|commit| commit.id == fix));
    assert!(
        analysis
            .commits
            .iter()
            .all(|commit| commit.gitmoji.shortcode != ":bookmark:")
    );
}

#[test]
fn initial_range_includes_the_root_and_pins_head() {
    let repository = GitRepo::new();
    let root = commit(&repository, 0, "✨ Root feature");
    let range = CommitRange::initial(repository.path(), "HEAD").unwrap();
    commit(&repository, 1, "💥 Later breaking change");

    let analysis = GitCliffAnalyzer.analyze(&range).unwrap();

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

    let analysis = GitCliffAnalyzer.analyze(&range).unwrap();

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

    let analysis = GitCliffAnalyzer.analyze(&range).unwrap();

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

    let error = RejectingAnalyzer.analyze(&range).unwrap_err();

    assert_eq!(error.to_string(), "external analyzer failure");
}
