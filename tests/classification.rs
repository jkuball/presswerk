mod support;

use presswerk::classification::{CommitClassification, classify_commit};
use presswerk::declaration::Tag;
use semver::Version;
use support::GitRepo;

fn classify(repository: &GitRepo, target: &str) -> CommitClassification {
    classify_commit(
        repository.path(),
        target,
        &Tag {
            format: "v{version}".into(),
        },
        |commit| Version::parse(&repository.version_at(commit)),
    )
    .unwrap()
}

#[test]
fn classifies_an_ordinary_commit_after_a_release() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Prepare release");
    repository.annotated_tag("v0.1.0");
    repository.write("README.md", "ordinary change\n");
    let ordinary = repository.commit("📝 Document release");

    assert_eq!(
        classify(&repository, &ordinary),
        CommitClassification::OrdinaryCommit
    );
}

#[test]
fn classifies_a_changed_untagged_version_as_a_new_release() {
    let repository = GitRepo::new();
    repository.commit_version("0.0.0", "🎉 Begin project");
    let release = repository.commit_version("0.1.0", "🔖 Prepare release");

    assert_eq!(
        classify(&repository, &release),
        CommitClassification::NewRelease
    );
}

#[test]
fn reports_the_release_commit_inherited_by_a_pending_fixup() {
    let repository = GitRepo::new();
    repository.commit_version("0.0.0", "🎉 Begin project");
    let release = repository.commit_version("0.1.0", "🔖 Prepare release");
    repository.write("fix-one", "one\n");
    repository.commit("🐛 First fixup");
    repository.write("fix-two", "two\n");
    let fixup = repository.commit("🐛 Second fixup");

    let classification = classify(&repository, &fixup);
    assert_eq!(
        classification,
        CommitClassification::PendingRelease {
            version: Version::new(0, 1, 0),
            commit: release.clone(),
        }
    );
    assert!(classification.to_string().contains(&release));
}

#[test]
fn peels_an_annotated_tag_for_a_retry_or_republish() {
    let repository = GitRepo::new();
    repository.commit_version("0.0.0", "🎉 Begin project");
    let release = repository.commit_version("0.1.0", "🔖 Prepare release");
    repository.annotated_tag("v0.1.0");

    assert_eq!(
        classify(&repository, &release),
        CommitClassification::RetryOrRepublish
    );
}

#[test]
fn rejects_an_unchanged_version_tagged_on_the_candidate() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🎉 Import project");
    repository.write("README.md", "ordinary change\n");
    let candidate = repository.commit("📝 Document project");
    repository.tag("v0.1.0");

    assert_eq!(
        classify(&repository, &candidate),
        CommitClassification::InvalidCandidate
    );
}

#[test]
fn reports_a_tag_on_a_non_ancestor_as_a_conflict() {
    let repository = GitRepo::new();
    repository.commit_version("0.0.0", "🎉 Begin project");
    let main_branch = repository.git(["branch", "--show-current"]);

    repository.git(["switch", "--quiet", "--create", "other"]);
    let tagged_commit = repository.commit_version("0.1.0", "🔖 Other release");
    repository.annotated_tag("v0.1.0");

    repository.git(["switch", "--quiet", &main_branch]);
    let candidate = repository.commit_version("0.1.0", "🔖 Candidate release");

    assert_eq!(
        classify(&repository, &candidate),
        CommitClassification::Conflict {
            tag: "v0.1.0".into(),
            tagged_commit,
        }
    );
}

#[test]
fn treats_an_untagged_zero_version_as_the_fresh_baseline() {
    let repository = GitRepo::new();
    let baseline = repository.commit_version("0.0.0", "🎉 Begin project");
    repository.tag("snapshot");

    assert_eq!(
        classify(&repository, &baseline),
        CommitClassification::OrdinaryCommit
    );
}

#[test]
fn accepts_a_revert_to_the_previous_tagged_version() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Prepare release");
    repository.tag("v0.1.0");
    repository.commit_version("0.2.0", "🔖 Prepare failed release");
    let revert = repository.commit_version("0.1.0", "⏪️ Revert failed release");

    assert_eq!(
        classify(&repository, &revert),
        CommitClassification::OrdinaryCommit
    );
}

#[test]
fn compares_a_merge_candidate_with_its_first_parent() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🎉 Begin project");
    let main_branch = repository.git(["branch", "--show-current"]);

    repository.git(["switch", "--quiet", "--create", "release"]);
    repository.commit_version("0.2.0", "🔖 Prepare release");

    repository.git(["switch", "--quiet", &main_branch]);
    repository.write("README.md", "main change\n");
    repository.commit("📝 Document project");
    repository.git(["merge", "--quiet", "--no-ff", "--no-edit", "release"]);
    let merge = repository.head();

    assert_eq!(
        classify(&repository, &merge),
        CommitClassification::NewRelease
    );
}
