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
        token.env = "GITHUB_TOKEN";
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
        token.env = "GITHUB_TOKEN";
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
    release_repository_with(RELEASE_FLAKE)
}

fn release_repository_with(flake: &str) -> GitRepo {
    let repository = GitRepo::new();
    write_cargo_version(&repository, "0.0.0");
    repository.write("flake.nix", flake);
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
fn plan_and_prepare_help_document_the_trusted_predecessor_boundary() {
    for command in ["plan", "prepare"] {
        let output = presswerk(&[command, "--help"]);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success());
        assert!(stdout.contains("--trust-predecessor <TAG=SHA>"), "{stdout}");
        assert!(stdout.contains("trustedPredecessor"), "{stdout}");
    }
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
fn publish_without_a_declared_secret_fails_before_writing_anything() {
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

    let output = presswerk_in_with(
        repository.path(),
        &["publish"],
        &[("GITHUB_TOKEN", Path::new(""))],
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "secret environment variable \"GITHUB_TOKEN\" for publishers.github-release.default.token is not set"
        ),
        "{stderr}"
    );
    assert_eq!(repository.git(["tag", "--list"]), "");
    assert_eq!(repository.git(["ls-remote", "--tags", &remote_path]), "");
}

/// A secret value that must never appear in what the CLI prints.
const SENTINEL: &str = "presswerk-sentinel-4711";

#[test]
fn plan_never_prints_a_secret_value() {
    let repository = release_repository();
    repository.write("other", "implemented\n");
    repository.commit(&format!("✨ Add {SENTINEL}"));

    let output = presswerk_in_with(
        repository.path(),
        &["plan"],
        &[("GITHUB_TOKEN", Path::new(SENTINEL))],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stdout.contains("✨ Add [redacted]"), "{stdout}");
    assert!(!stdout.contains(SENTINEL) && !stderr.contains(SENTINEL));
}

#[test]
fn publish_errors_never_print_a_secret_value() {
    let repository = release_repository();
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    // The remote is unreachable, and Git names it in the error.
    repository.git([
        "remote",
        "add",
        "origin",
        &format!("/nonexistent/{SENTINEL}"),
    ]);

    let output = presswerk_in_with(
        repository.path(),
        &["publish"],
        &[("GITHUB_TOKEN", Path::new(SENTINEL))],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("/nonexistent/[redacted]"), "{stderr}");
    assert!(!stdout.contains(SENTINEL) && !stderr.contains(SENTINEL));
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
            // Every declared secret is checked first; this test is about the identity.
            ("GITHUB_TOKEN", Path::new("presswerk-test-token")),
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

/// Declares `trustedPredecessor` in the adopted repository's release module, in a new commit.
fn declare_trusted_predecessor(repository: &GitRepo, tag: &str, commit: &str) {
    let flake = RELEASE_FLAKE.replace(
        r#"changelog.path = "CHANGELOG.md";"#,
        &format!(
            "changelog.path = \"CHANGELOG.md\";\n      trustedPredecessor = {{ tag = \"{tag}\"; commit = \"{commit}\"; }};"
        ),
    );
    assert_ne!(flake, RELEASE_FLAKE);
    repository.write("flake.nix", flake);
    repository.commit("🔧 Trust the previous release");
}

#[test]
fn plan_and_prepare_adopt_a_release_trusted_in_the_release_module() {
    let (repository, predecessor, _remote) = adopted_repository(false);
    declare_trusted_predecessor(&repository, "v0.38.2", &predecessor);

    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert!(stdout.contains("next version       0.39.0\n"), "{stdout}");
    assert!(
        stdout.ends_with("\nNothing written. Run `presswerk prepare` to commit 0.39.0.\n"),
        "{stdout}"
    );

    let prepared = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&prepared.stderr);
    assert!(prepared.status.success(), "{stderr}");
    assert!(!stderr.contains("trustedPredecessor"), "{stderr}");
    let release = repository.head();
    assert!(
        repository
            .git(["log", "-1", "--format=%B"])
            .ends_with(&format!(
                "Presswerk-Trusted-Predecessor: v0.38.2 {predecessor}"
            ))
    );

    // Above the release it was meant for, the entry is stale: harmless, with a warning.
    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    let stderr = String::from_utf8_lossy(&plan.stderr);
    assert!(plan.status.success(), "{stderr}");
    assert!(
        stdout.contains(&format!(
            "previous release   v0.39.0 ({release}, pending: not published yet)\n"
        )),
        "{stdout}"
    );
    assert!(
        stderr.contains(&format!(
            "warning: trustedPredecessor v0.38.2={predecessor} names an older release than the previous release v0.39.0; it has no effect and can be removed\n"
        )),
        "{stderr}"
    );
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&prepared.stderr);
    assert!(prepared.status.success(), "{stderr}");
    assert!(
        stderr.contains("it has no effect and can be removed"),
        "{stderr}"
    );
    assert_eq!(repository.head(), release);
}

#[test]
fn plan_and_prepare_accept_the_same_trust_from_flag_and_module() {
    let (repository, predecessor, _remote) = adopted_repository(false);
    declare_trusted_predecessor(&repository, "v0.38.2", &predecessor);
    let trust = format!("v0.38.2={predecessor}");

    for command in ["plan", "prepare"] {
        let output = presswerk_in(repository.path(), &[command, "--trust-predecessor", &trust]);
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        repository
            .git(["log", "-1", "--format=%B"])
            .ends_with(&format!(
                "Presswerk-Trusted-Predecessor: v0.38.2 {predecessor}"
            ))
    );
}

#[test]
fn plan_and_prepare_refuse_different_trust_from_flag_and_module() {
    let (repository, predecessor, _remote) = adopted_repository(false);
    declare_trusted_predecessor(&repository, "v0.38.2", &predecessor);
    let head = repository.head();
    let trust = format!("v0.38.2={head}");

    for command in ["plan", "prepare"] {
        let output = presswerk_in(repository.path(), &[command, "--trust-predecessor", &trust]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{command}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "--trust-predecessor v0.38.2={head} differs from trustedPredecessor v0.38.2={predecessor} in the release declaration"
            )),
            "{command}: {stderr}"
        );
    }
    assert_eq!(repository.head(), head);
}

#[test]
fn plan_and_prepare_refuse_a_trusted_predecessor_whose_tag_moved() {
    let (repository, predecessor, _remote) = adopted_repository(false);
    let declared = repository.git(["rev-parse", "HEAD"]);
    declare_trusted_predecessor(&repository, "v0.38.2", &declared);
    let head = repository.head();

    for command in ["plan", "prepare"] {
        let output = presswerk_in(repository.path(), &[command]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{command}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "trusted predecessor v0.38.2={declared} does not match expected predecessor v0.38.2={predecessor}"
            )),
            "{command}: {stderr}"
        );
    }
    assert_eq!(repository.head(), head);
}

#[test]
fn plan_and_prepare_refuse_a_trusted_predecessor_outside_the_history() {
    let (repository, _predecessor, _remote) = adopted_repository(false);
    let unrelated = "d1d91d0afbc0840d2f9b4c60321acbc8e1b7ce6b";
    declare_trusted_predecessor(&repository, "v0.38.1", unrelated);
    let head = repository.head();

    for command in ["plan", "prepare"] {
        let output = presswerk_in(repository.path(), &[command]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{command}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "trustedPredecessor v0.38.1={unrelated} in the release declaration names neither the previous release v0.38.2 nor an older one"
            )),
            "{command}: {stderr}"
        );
    }
    assert_eq!(repository.head(), head);
}

#[test]
fn plan_crosses_an_untrusted_predecessor_with_the_flag() {
    let (repository, predecessor, _remote) = adopted_repository(false);

    let refused = presswerk_in(repository.path(), &["plan"]);
    assert!(!refused.status.success());

    let trust = format!("v0.38.2={predecessor}");
    let plan = presswerk_in(repository.path(), &["plan", "--trust-predecessor", &trust]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert!(stdout.contains("next version       0.39.0\n"), "{stdout}");
    assert!(
        stdout.ends_with(&format!(
            "\nNothing written. Run `presswerk prepare --trust-predecessor {trust}` to commit 0.39.0.\n"
        )),
        "{stdout}"
    );
}

#[test]
fn a_predecessor_with_the_old_secret_shape_needs_trust() {
    predecessor_this_cli_cannot_read_needs_trust(
        &RELEASE_FLAKE.replace("token.env = ", "token.secret = "),
        "unknown variant `secret`, expected `value` or `env`",
    );
}

#[test]
fn a_predecessor_with_the_nested_secret_shape_needs_trust() {
    predecessor_this_cli_cannot_read_needs_trust(
        &RELEASE_FLAKE.replace("token.env = ", "token.secret.env = "),
        "unknown variant `secret`, expected `value` or `env`",
    );
}

#[test]
fn a_predecessor_with_an_unknown_publisher_type_needs_trust() {
    predecessor_this_cli_cannot_read_needs_trust(
        &RELEASE_FLAKE.replace(
            "publishers.\"github-release\".default = {",
            "publishers.npm.default = { };\n      publishers.\"github-release\".default = {",
        ),
        "unknown publisher type \"npm\"",
    );
}

/// A previous release whose declaration this CLI cannot read stops `prepare` with the reason and
/// the trust command, and the command crosses it.
fn predecessor_this_cli_cannot_read_needs_trust(predecessor_flake: &str, reason: &str) {
    let repository = GitRepo::new();
    write_cargo_version(&repository, "0.1.0");
    repository.write("flake.nix", predecessor_flake);
    repository.write("flake.lock", LOCK_FILE);
    repository.write(
        "CHANGELOG.md",
        "# Changelog\n\n## [0.1.0] - 2026-11-10\n\n- Old\n",
    );
    let predecessor = repository.commit("🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("flake.nix", RELEASE_FLAKE);
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add a feature");
    let remote = tempfile::tempdir().unwrap();
    repository.git(["init", "--quiet", "--bare", remote.path().to_str().unwrap()]);
    repository.git(["remote", "add", "origin", remote.path().to_str().unwrap()]);
    repository.git([
        "push",
        "--quiet",
        "origin",
        "HEAD:refs/heads/main",
        "v0.1.0",
    ]);

    let refused = presswerk_in(repository.path(), &["prepare"]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(
        stderr.contains("uses a declaration with schemaVersion 0 that this CLI cannot read"),
        "{stderr}"
    );
    assert!(stderr.contains(reason), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "presswerk prepare --trust-predecessor v0.1.0={predecessor}"
        )),
        "{stderr}"
    );

    let trust = format!("v0.1.0={predecessor}");
    let prepared = presswerk_in(
        repository.path(),
        &["prepare", "--trust-predecessor", &trust],
    );
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
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
    pending_repository_with(RELEASE_FLAKE)
}

fn pending_repository_with(flake: &str) -> (GitRepo, String, tempfile::TempDir) {
    let repository = release_repository_with(flake);
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
    // A template that needs a commit renders for the release, but not for the empty range above it.
    let flake = RELEASE_FLAKE.replace(
        r#"changelog.path = "CHANGELOG.md";"#,
        r#"changelog = {
        path = "CHANGELOG.md";
        template = "{{ groups[0].commits[0].message }}\n";
      };"#,
    );
    assert_ne!(flake, RELEASE_FLAKE);
    let (repository, pending, _remote) = pending_repository_with(&flake);

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
            "trusted predecessor is unnecessary: pending release v0.1.0 at {} is not published yet",
            &pending[..7]
        )),
        "{stderr}"
    );
    assert_eq!(repository.head(), merge);
}

#[test]
fn a_release_tagged_on_only_one_side_is_not_pending() {
    // Without publishers, a published predecessor is complete once its tag is on the remote.
    let flake = RELEASE_FLAKE.replace(
        r#"publishers."github-release".default = {
        apiUrl = "https://api.github.com";
        repository = "owner/project";
        token.env = "GITHUB_TOKEN";
      };"#,
        r#"publishers."github-release" = { };"#,
    );
    assert_ne!(flake, RELEASE_FLAKE);
    let (repository, pending, _remote) = pending_repository_with(&flake);
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

    // Published, but the tag was not fetched: the remote tag stands in for it.
    repository.git(["push", "--quiet", "origin", "v0.1.0"]);
    repository.git(["tag", "--delete", "v0.1.0"]);
    let plan = presswerk_in(repository.path(), &["plan"]);
    let stdout = String::from_utf8_lossy(&plan.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert!(
        stdout.contains(&format!("previous release   v0.1.0 ({pending})\n")),
        "{stdout}"
    );
    assert!(stdout.contains("analyzed commits   1\n"), "{stdout}");
    assert!(stdout.contains("✨ Add the second feature"), "{stdout}");
    assert!(stdout.contains("next version       0.2.0\n"), "{stdout}");

    let prepared = presswerk_in(repository.path(), &["prepare"]);
    let stdout = String::from_utf8_lossy(&prepared.stdout);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert!(stdout.starts_with("Preparing v0.2.0 on "), "{stdout}");
    assert_eq!(repository.git(["tag", "--list"]), "");
}

#[test]
fn prepare_refuses_a_pending_base_before_computing_the_next_release() {
    let (repository, pending, _remote) = pending_repository();
    let changelog = std::fs::read_to_string(repository.path().join("CHANGELOG.md")).unwrap();
    // The next release could not be prepared anyway; the pending release is the reason to report.
    repository.write(
        "CHANGELOG.md",
        changelog.replacen("## [0.1.0]", "## [0.2.0] - 2026-11-19\n\n## [0.1.0]", 1),
    );
    repository.write("second", "implemented\n");
    let head = repository.commit("✨ Add the second feature");

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
    assert_eq!(repository.head(), head);

    // `plan` keeps validating the next release.
    let plan = presswerk_in(repository.path(), &["plan"]);
    let stderr = String::from_utf8_lossy(&plan.stderr);
    assert!(!plan.status.success(), "{stderr}");
    assert!(
        stderr.contains("changelog already contains version 0.2.0"),
        "{stderr}"
    );
}
