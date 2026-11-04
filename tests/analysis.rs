mod support;

use presswerk::analysis::{Analyzer, Bump, CommitRange, GitCliffAnalyzer};
use support::GitRepo;

fn commit(repository: &GitRepo, sequence: usize, subject: &str) -> String {
    repository.write("history", format!("{sequence}\n"));
    repository.commit(subject)
}

#[test]
fn analyzes_initial_history_and_omits_non_entries() {
    let repository = GitRepo::new();
    commit(&repository, 0, "🔖 Previous release");
    let feature = commit(&repository, 1, "✨ Add release planning");
    let documentation = commit(&repository, 2, ":memo: Document release planning");
    commit(&repository, 3, "Merge pull request without Gitmoji");
    let fix = commit(&repository, 4, ":bug: Fix release planning");

    let analysis = GitCliffAnalyzer
        .analyze(&CommitRange::initial(repository.path(), repository.head()))
        .unwrap();

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
fn honors_the_base_while_analyzing_the_full_range() {
    let repository = GitRepo::new();
    commit(&repository, 0, "💥 Old breaking change");
    let base = commit(&repository, 1, "🔖 Previous release");
    let feature = commit(&repository, 2, "✨ Add range analysis");
    let target = commit(&repository, 3, "Merge pull request without Gitmoji");

    let analysis = GitCliffAnalyzer
        .analyze(&CommitRange::between(repository.path(), base, target))
        .unwrap();

    assert_eq!(analysis.bump, Some(Bump::Minor));
    assert_eq!(analysis.commits.len(), 1);
    assert_eq!(analysis.commits[0].id, feature);
}
