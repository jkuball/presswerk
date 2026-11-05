mod support;

use support::GitRepo;

#[test]
fn builds_repositories_with_versions_and_tags() {
    let repository = GitRepo::new();
    let first = repository.commit_version("0.0.0", "🎉 Begin project");
    repository.annotated_tag("v0.0.0");
    let second = repository.commit_version("0.1.0", "🔖 Release v0.1.0");

    assert_eq!(
        repository.git(["config", "--get", "commit.gpgSign"]),
        "false"
    );
    assert_eq!(repository.git(["config", "--get", "tag.gpgSign"]), "false");
    assert_eq!(
        repository.git(["config", "--get", "core.hooksPath"]),
        ".git/disabled-hooks"
    );
    assert_eq!(repository.version_at(&first), "0.0.0");
    assert_eq!(repository.version_at(&second), "0.1.0");
    assert_eq!(repository.git(["rev-parse", "v0.0.0^{commit}"]), first);
}
