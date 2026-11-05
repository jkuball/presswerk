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
      schemaVersion = 1;
      version = "0.0.0";
      versionFiles = [];
      tag.format = "v{version}";
      analyzer.convention = "gitmoji";
      changelog.path = "CHANGELOG.md";
      publishers."github-release".default = {
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
fn version_flag_prints_cargo_version() {
    let output = presswerk(&["--version"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "presswerk 0.0.0\n");
}
