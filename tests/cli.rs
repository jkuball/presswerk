mod support;

use std::path::Path;
use std::process::{Command, Output};
use support::GitRepo;

const LOCK_FILE: &str = r#"{
  "nodes": {
    "root": {}
  },
  "root": "root",
  "version": 7
}
"#;

fn presswerk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_presswerk"))
        .args(args)
        .output()
        .expect("failed to run presswerk")
}

fn presswerk_in(repository: &Path, args: &[&str]) -> Output {
    presswerk_in_with(repository, args, &[])
}

fn presswerk_in_with(repository: &Path, args: &[&str], envs: &[(&str, &Path)]) -> Output {
    // Give each CLI test a writable store without touching the host's global Nix state.
    let store = tempfile::tempdir().expect("create temporary Nix store");
    Command::new(env!("CARGO_BIN_EXE_presswerk"))
        .args(args)
        .current_dir(repository)
        .envs(envs.iter().copied())
        .env(
            "NIX_REMOTE",
            format!(
                "local?root={}",
                store.path().canonicalize().unwrap().display()
            ),
        )
        .output()
        .expect("failed to run presswerk")
}

fn ordinary_repository() -> GitRepo {
    let repository = GitRepo::new();
    repository.write(
        "flake.nix",
        r#"{
  outputs = { self }: {
    presswerk.default = {
      schemaVersion = 0;
      version = "0.0.0";
      stable = false;
      versionFiles = [];
      tag.format = "v{version}";
      analyzer = {
        convention = "gitmoji";
        gitmoji.catalog = {};
      };
      changelog.path = "CHANGELOG.md";
      publishers."github-release".default = {
        apiUrl = "https://api.github.com";
        repository = "owner/project";
        token.secret = "GITHUB_TOKEN";
      };
    };
  };
}
"#,
    );
    repository.write("flake.lock", LOCK_FILE);
    repository.commit("🎉 Begin project");
    repository
}

const RELEASE_FLAKE: &str = r#"{
  outputs = { self }: {
    presswerk.default = {
      schemaVersion = 0;
      version = (builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"))).package.version;
      stable = false;
      versionFiles = [
        { path = "Cargo.toml"; toml = "package.version"; }
        { path = "Cargo.lock"; cargoLock = "example"; }
      ];
      tag.format = "v{version}";
      analyzer = {
        convention = "gitmoji";
        gitmoji.catalog.sparkles = {
          emoji = "✨";
          description = "Introduce new features.";
          semver = "minor";
        };
      };
      changelog.path = "CHANGELOG.md";
      publishers."github-release".default = {
        apiUrl = "https://api.github.com";
        repository = "owner/project";
        token.secret = "GITHUB_TOKEN";
      };
    };
  };
}
"#;

fn write_cargo_version(repository: &GitRepo, version: &str) {
    repository.write(
        "Cargo.toml",
        format!("[package]\nname = \"example\"\nversion = \"{version}\"\n"),
    );
    repository.write(
        "Cargo.lock",
        format!("version = 4\n\n[[package]]\nname = \"example\"\nversion = \"{version}\"\n"),
    );
}

fn release_repository() -> GitRepo {
    let repository = GitRepo::new();
    write_cargo_version(&repository, "0.0.0");
    repository.write("flake.nix", RELEASE_FLAKE);
    repository.write("flake.lock", LOCK_FILE);
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add the first feature");
    repository
}

#[test]
fn no_arguments_prints_help() {
    let output = presswerk(&[]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: presswerk"));
}

#[test]
fn help_flag_prints_the_publish_command() {
    let output = presswerk(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: presswerk"));
    assert!(stdout.contains("publish"));
}

#[test]
fn publish_help_documents_the_optional_revision_and_options() {
    let output = presswerk(&["publish", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: presswerk publish [OPTIONS] [REVISION]"));
    assert!(stdout.contains("--fail-fast"));
    assert!(stdout.contains("--jobs <N>"));
    assert!(stdout.contains("default: 8"));
    assert!(stdout.contains("default: HEAD"));
}

#[test]
fn prepare_help_documents_the_trusted_predecessor_boundary() {
    let output = presswerk(&["prepare", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--trust-predecessor <TAG=SHA>"));
}

#[test]
fn publish_defaults_to_head_and_exits_for_an_ordinary_commit() {
    let repository = ordinary_repository();

    let output = presswerk_in(repository.path(), &["publish"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let head = repository.head();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Nothing to publish: {} is not a release commit.\n",
            &head[..7]
        )
    );
}

#[test]
fn help_flag_prints_plan_and_prepare_commands() {
    let output = presswerk(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for command in ["plan", "prepare"] {
        assert!(stdout.contains(command), "help did not contain {command}");
    }
}

#[test]
fn plan_reads_head_and_prepare_creates_the_first_release_commit() {
    let repository = release_repository();
    repository.write("Cargo.toml", "uncommitted and invalid\n");

    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    let stderr = String::from_utf8_lossy(&plan.stderr);
    assert!(plan.status.success(), "{stderr}");
    assert!(stderr.contains("working tree is dirty"));
    assert!(stdout.contains("previous release   none (implicit 0.0.0 baseline)"));
    assert!(stdout.contains("next version       0.1.0"));
    assert!(stdout.contains("Cargo.toml  0.0.0 -> 0.1.0"));
    assert!(stdout.contains("✨ Add the first feature"));

    let refused = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(
        stderr.contains("local changes in Cargo.toml would be overwritten"),
        "{stderr}"
    );

    repository.git(["checkout", "--", "Cargo.toml"]);
    repository.write("scratch", "untracked\n");
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&prepared.stderr);
    assert!(prepared.status.success(), "{stderr}");
    assert!(
        stderr.contains("warning: working tree is dirty; preparing HEAD"),
        "{stderr}"
    );
    assert_eq!(repository.git(["status", "--short"]), "?? scratch");
    let stdout = String::from_utf8_lossy(&prepared.stdout);
    assert!(stdout.starts_with("Preparing v0.1.0 on "), "{stdout}");
    assert!(stdout.contains("Cargo.toml"), "{stdout}");
    assert!(stdout.contains("0.0.0 -> 0.1.0"), "{stdout}");
    assert!(stdout.contains("added 0.1.0"), "{stdout}");
    let release = repository.head();
    assert!(
        repository
            .git(["show", &format!("{release}:Cargo.toml")])
            .contains("version = \"0.1.0\"")
    );
    assert!(
        repository
            .git(["show", &format!("{release}:Cargo.lock")])
            .contains("version = \"0.1.0\"")
    );
    let changelog = repository.git(["show", &format!("{release}:CHANGELOG.md")]);
    assert!(
        changelog.contains("### ✨ Introduce new features\n\n- Add the first feature"),
        "{changelog}"
    );
}

#[test]
fn publish_without_a_git_identity_fails_before_writing_anything() {
    let repository = release_repository();
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    repository.git(["config", "--unset", "user.name"]);
    repository.git(["config", "--unset", "user.email"]);
    // Keep the host's global and system Git configuration, and with it any identity, out of reach.
    let home = tempfile::tempdir().unwrap();
    let nowhere = home.path().join("missing");

    let output = presswerk_in_with(
        repository.path(),
        &["publish"],
        &[
            ("HOME", home.path()),
            ("XDG_CONFIG_HOME", home.path()),
            ("GIT_CONFIG_GLOBAL", &nowhere),
            ("GIT_CONFIG_NOSYSTEM", Path::new("1")),
        ],
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("could not read Git tagger configuration"),
        "{stderr}"
    );
    assert!(
        stderr.contains("set them with `git config` in the job that runs `publish`"),
        "{stderr}"
    );
    assert_eq!(repository.git(["tag", "--list"]), "");
    let remote_tags = Command::new("git")
        .args(["--git-dir", &remote_path, "tag", "--list"])
        .output()
        .unwrap();
    assert!(remote_tags.status.success());
    assert!(remote_tags.stdout.is_empty());
}

/// A repository whose release v0.38.2 was made by another tool, followed by the Presswerk declaration.
fn adopted_repository(lightweight_tag: bool) -> (GitRepo, String, tempfile::TempDir) {
    let repository = GitRepo::new();
    write_cargo_version(&repository, "0.38.2");
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## v0.38.2 (2026-11-10)\n\n- Old\n",
    );
    let predecessor = repository.commit("chore(release): 0.38.2");
    if lightweight_tag {
        repository.tag("v0.38.2");
    } else {
        repository.annotated_tag("v0.38.2");
    }
    repository.write("flake.nix", RELEASE_FLAKE);
    repository.write("flake.lock", LOCK_FILE);
    repository.write("feature", "implemented\n");
    repository.commit("✨ Release with Presswerk");
    let remote = tempfile::tempdir().unwrap();
    repository.git(["init", "--quiet", "--bare", remote.path().to_str().unwrap()]);
    repository.git(["remote", "add", "origin", remote.path().to_str().unwrap()]);
    repository.git([
        "push",
        "--quiet",
        "origin",
        "HEAD:refs/heads/main",
        "v0.38.2",
    ]);
    (repository, predecessor, remote)
}

#[test]
fn prepare_adopts_a_release_of_another_tool_once_it_is_trusted() {
    let (repository, predecessor, _remote) = adopted_repository(false);
    let command = format!("presswerk prepare --trust-predecessor v0.38.2={predecessor}");

    let refused = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(
        stderr.contains(&format!(
            "previous release v0.38.2 at {predecessor} has no Presswerk declaration"
        )),
        "{stderr}"
    );
    assert!(stderr.contains(&command), "{stderr}");

    let trust = format!("v0.38.2={predecessor}");
    let prepared = presswerk_in(
        repository.path(),
        &["prepare", "--trust-predecessor", &trust],
    );
    let stderr = String::from_utf8_lossy(&prepared.stderr);
    assert!(prepared.status.success(), "{stderr}");
    let message = repository.git(["log", "-1", "--format=%B"]);
    assert!(
        message.ends_with(&format!(
            "Presswerk-Trusted-Predecessor: v0.38.2 {predecessor}"
        )),
        "{message}"
    );
    let release = repository.head();
    assert!(
        repository
            .git(["show", &format!("{release}:Cargo.toml")])
            .contains("version = \"0.39.0\"")
    );
}

#[test]
fn prepare_adopts_a_lightweight_tag_of_another_tool() {
    let (repository, predecessor, _remote) = adopted_repository(true);

    let trust = format!("v0.38.2={predecessor}");
    let prepared = presswerk_in(
        repository.path(),
        &["prepare", "--trust-predecessor", &trust],
    );
    let stderr = String::from_utf8_lossy(&prepared.stderr);
    assert!(prepared.status.success(), "{stderr}");
    assert!(
        repository
            .git(["log", "-1", "--format=%B"])
            .ends_with(&format!(
                "Presswerk-Trusted-Predecessor: v0.38.2 {predecessor}"
            ))
    );
}

#[test]
fn version_flag_prints_cargo_version() {
    let output = presswerk(&["--version"]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("presswerk {}\n", env!("CARGO_PKG_VERSION"))
    );
}

/// A repository whose first release commit v0.1.0 is prepared and pushed but not published, with
/// the remote that lacks its tag.
fn pending_repository() -> (GitRepo, String, tempfile::TempDir) {
    let repository = release_repository();
    let remote = tempfile::tempdir().unwrap();
    repository.git(["init", "--quiet", "--bare", remote.path().to_str().unwrap()]);
    repository.git(["remote", "add", "origin", remote.path().to_str().unwrap()]);
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let pending = repository.head();
    repository.git(["push", "--quiet", "origin", "HEAD:refs/heads/main"]);
    (repository, pending, remote)
}

#[test]
fn prepare_has_nothing_to_do_at_a_pending_release() {
    let (repository, pending, _remote) = pending_repository();

    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert!(
        stdout.contains(&format!(
            "previous release   v0.1.0 ({pending}, pending: not published yet)\n"
        )),
        "{stdout}"
    );
    assert!(stdout.contains("analyzed commits   0\n"), "{stdout}");
    assert!(
        stdout.contains("next version       none (no release-worthy commits)\n"),
        "{stdout}"
    );

    let prepared = presswerk_in(repository.path(), &["prepare"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&prepared.stdout),
        format!(
            "Nothing to prepare: pending release v0.1.0 at {} is not published yet.\n",
            &pending[..7]
        )
    );
    assert_eq!(repository.head(), pending);
    assert_eq!(repository.git(["status", "--short"]), "");
}

#[test]
fn plan_builds_on_a_pending_release_that_prepare_refuses_to_build_on() {
    let (repository, pending, _remote) = pending_repository();
    // A pull request branched off before the release commit, merged the way pull request CI does.
    repository.git(["checkout", "--quiet", "-b", "feature", "HEAD~1"]);
    repository.write("second", "implemented\n");
    repository.commit("✨ Add the second feature");
    repository.write("third", "implemented\n");
    repository.commit("✨ Add the third feature");
    repository.git(["checkout", "--quiet", "-"]);
    repository.git([
        "merge",
        "--quiet",
        "--no-ff",
        "--message",
        "🔀 Merge the feature",
        "feature",
    ]);
    let merge = repository.head();

    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert!(
        stdout.contains(&format!(
            "previous release   v0.1.0 ({pending}, pending: not published yet)\n"
        )),
        "{stdout}"
    );
    assert!(stdout.contains("analyzed commits   2\n"), "{stdout}");
    assert!(stdout.contains("✨ Add the second feature"), "{stdout}");
    assert!(stdout.contains("✨ Add the third feature"), "{stdout}");
    assert!(!stdout.contains("✨ Add the first feature"), "{stdout}");
    assert!(stdout.contains("next version       0.2.0\n"), "{stdout}");
    assert!(stdout.ends_with("\nNothing written.\n"), "{stdout}");

    let refused = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: pending release v0.1.0 at {} is not published yet; publish it before preparing the next one\n",
            &pending[..7]
        )
    );
    assert_eq!(repository.head(), merge);

    let trust = format!("v0.1.0={pending}");
    let trusted = presswerk_in(
        repository.path(),
        &["prepare", "--trust-predecessor", &trust],
    );
    let stderr = String::from_utf8_lossy(&trusted.stderr);
    assert!(!trusted.status.success(), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "--trust-predecessor is unnecessary: pending release v0.1.0 at {} is not published yet",
            &pending[..7]
        )),
        "{stderr}"
    );
    assert_eq!(repository.head(), merge);
}

#[test]
fn a_release_tagged_on_only_one_side_is_not_pending() {
    let (repository, pending, _remote) = pending_repository();
    repository.write("second", "implemented\n");
    repository.commit("✨ Add the second feature");

    // Tagged locally, but the push of the tag failed.
    repository.git([
        "tag",
        "--annotate",
        "--message",
        "v0.1.0",
        "v0.1.0",
        &pending,
    ]);
    let plan = presswerk_in(repository.path(), &["plan"]);
    let stderr = String::from_utf8_lossy(&plan.stderr);
    assert!(!plan.status.success(), "{stderr}");
    assert!(
        stderr.contains("previous release tag \"v0.1.0\" is missing from the remote"),
        "{stderr}"
    );

    // Published, but the tag was not fetched.
    repository.git(["push", "--quiet", "origin", "v0.1.0"]);
    repository.git(["tag", "--delete", "v0.1.0"]);
    let plan = presswerk_in(repository.path(), &["plan"]);
    let stderr = String::from_utf8_lossy(&plan.stderr);
    assert!(!plan.status.success(), "{stderr}");
    assert!(
        stderr.contains("previous release tag \"v0.1.0\" is missing\n"),
        "{stderr}"
    );
}
