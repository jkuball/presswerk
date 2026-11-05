mod support;

use git2::Oid;
use presswerk::release_commit::{ReleaseCommitError, ReleaseFile, create_release_commit};
use semver::Version;
use support::GitRepo;

#[test]
fn creates_a_bookmark_commit_from_the_parent_tree_without_using_the_index() {
    let repository = GitRepo::new();
    repository.write(
        "Cargo.toml",
        "[package]\nname = \"example\"\nversion = \"0.0.0\"\n",
    );
    repository.write("README.md", "committed\n");
    let parent = repository.commit("🎉 Begin project");
    let branch = repository.git(["symbolic-ref", "HEAD"]);

    repository.write("README.md", "staged but not released\n");
    repository.write("secret.txt", "must not leak\n");
    repository.git(["add", "README.md", "secret.txt"]);

    let release = create_release_commit(
        repository.path(),
        &branch,
        Oid::from_str(&parent).unwrap(),
        &Version::new(0, 1, 0),
        &[
            ReleaseFile::new(
                "Cargo.toml",
                "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
            ),
            ReleaseFile::new(
                "docs/CHANGELOG.md",
                "# Changelog\n\n## [0.1.0] - 2026-11-04\n\nFirst release.\n",
            ),
        ],
    )
    .unwrap();

    assert_eq!(repository.head(), release.to_string());
    assert_eq!(
        repository.git(["show", "--format=", &format!("{release}:README.md")]),
        "committed"
    );
    assert_eq!(
        repository.git(["show", "--format=", &format!("{release}:Cargo.toml")]),
        "[package]\nname = \"example\"\nversion = \"0.1.0\""
    );
    assert_eq!(
        repository.git(["show", "--format=%s", "--no-patch", &release.to_string()]),
        "🔖 Prepare release 0.1.0"
    );
    assert_eq!(
        repository.git(["rev-parse", &format!("{release}^")]),
        parent
    );
    assert!(
        !repository
            .git_output(["cat-file", "-e", &format!("{release}:secret.txt")])
            .status
            .success()
    );
}

#[test]
fn keeps_a_branch_that_no_longer_points_to_the_expected_parent() {
    let repository = GitRepo::new();
    repository.write("VERSION", "0.0.0\n");
    let expected = repository.commit("🎉 Begin project");
    let branch = repository.git(["symbolic-ref", "HEAD"]);
    repository.write("README.md", "branch moved\n");
    let actual = repository.commit("📝 Move branch");

    let error = create_release_commit(
        repository.path(),
        &branch,
        Oid::from_str(&expected).unwrap(),
        &Version::new(0, 1, 0),
        &[ReleaseFile::new("VERSION", "0.1.0\n")],
    )
    .unwrap_err();

    let ReleaseCommitError::BranchMoved {
        expected: found_expected,
        actual: found_actual,
        commit,
        ..
    } = error
    else {
        panic!("expected moved branch error")
    };
    assert_eq!(found_expected.to_string(), expected);
    assert_eq!(found_actual.unwrap().to_string(), actual);
    assert_eq!(repository.head(), actual);
    assert_eq!(
        repository.git(["cat-file", "-t", &commit.to_string()]),
        "commit"
    );
    assert_eq!(
        repository.git(["rev-parse", &format!("{commit}^")]),
        expected
    );
}
