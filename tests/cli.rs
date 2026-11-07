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
    // Give each CLI test a writable store without touching the host's global Nix state.
    let store = tempfile::tempdir().expect("create temporary Nix store");
    Command::new(env!("CARGO_BIN_EXE_presswerk"))
        .args(args)
        .current_dir(repository)
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
      schemaVersion = 2;
      version = "0.0.0";
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

fn release_repository() -> GitRepo {
    let repository = GitRepo::new();
    repository.write(
        "Cargo.toml",
        "[package]\nname = \"example\"\nversion = \"0.0.0\"\n",
    );
    repository.write(
        "Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"example\"\nversion = \"0.0.0\"\n",
    );
    repository.write(
        "flake.nix",
        r#"{
  outputs = { self }: {
    presswerk.default = {
      schemaVersion = 2;
      version = (builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"))).package.version;
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
"#,
    );
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
fn publish_help_documents_the_optional_revision() {
    let output = presswerk(&["publish", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: presswerk publish [REVISION]"));
    assert!(stdout.contains("default: HEAD"));
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Nothing to do.\n");
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

    repository.git(["checkout", "--", "Cargo.toml"]);
    let prepared = presswerk_in(repository.path(), &["prepare"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert!(String::from_utf8_lossy(&prepared.stdout).contains("Release v0.1.0"));
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
    assert!(
        repository
            .git(["show", &format!("{release}:CHANGELOG.md")])
            .contains("✨ Add the first feature")
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
