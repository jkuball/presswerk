mod support;

use presswerk::declaration::DeclarationError;
use presswerk::evaluation::{
    EvaluatedReleaseDeclaration, EvaluationError, ReleaseDeclarationEvaluator,
};
use semver::Version;
use std::fs;
use support::GitRepo;

const LOCK_FILE: &str = r#"{
  "nodes": {
    "root": {}
  },
  "root": "root",
  "version": 7
}
"#;

fn valid_flake(version: &str) -> String {
    r#"{
  outputs = { self }: {
    packages.x86_64-linux.presswerk = throw "installable lookup must not be used";
    legacyPackages.x86_64-linux.presswerk = throw "installable lookup must not be used";

    presswerk.default = {
      schemaVersion = 1;
      version = "@VERSION@";
      versionFiles = [
        { path = "Cargo.toml"; toml = "package.version"; }
        { path = "Cargo.lock"; cargoLock = "presswerk"; }
      ];
      tag.format = "v{version}";
      analyzer.convention = "gitmoji";
      changelog.path = "CHANGELOG.md";
      publishers."github-release".default = {
        repository = "jkuball/presswerk";
        token.secret = "GITHUB_TOKEN";
      };
    };
  };
}
"#
    .replace("@VERSION@", version)
}

fn flake(output: &str) -> String {
    format!("{{\n  outputs = {{ self }}: {{\n    presswerk.default = {output};\n  }};\n}}\n")
}

fn evaluate(
    repository: &GitRepo,
    revision: &str,
) -> Result<EvaluatedReleaseDeclaration, EvaluationError> {
    let store = tempfile::tempdir().unwrap();
    ReleaseDeclarationEvaluator::with_store_url(format!(
        "local?root={}",
        store.path().canonicalize().unwrap().display()
    ))
    .evaluate(repository.path(), revision)
}

fn locked_repository(version: &str) -> (GitRepo, String) {
    let repository = GitRepo::new();
    repository.write("flake.nix", valid_flake(version));
    repository.write("flake.lock", LOCK_FILE);
    let commit = repository.commit("🎉 Add release declaration");
    (repository, commit)
}

#[test]
fn evaluates_only_the_requested_commit_despite_working_tree_and_index_changes() {
    let (repository, commit) = locked_repository("1.2.3");

    repository.write("flake.nix", valid_flake("9.9.9"));
    repository.write("flake.lock", "not the committed lock\n");
    let working_lock = fs::read(repository.path().join("flake.lock")).unwrap();
    let unstaged = evaluate(&repository, &commit).unwrap();
    assert_eq!(unstaged.commit.to_string(), commit);
    assert_eq!(unstaged.declaration.version, Version::new(1, 2, 3));

    repository.git(["add", "flake.nix", "flake.lock"]);
    let staged = evaluate(&repository, "HEAD").unwrap();
    assert_eq!(staged.commit.to_string(), commit);
    assert_eq!(staged.declaration.version, Version::new(1, 2, 3));
    assert_eq!(
        fs::read(repository.path().join("flake.lock")).unwrap(),
        working_lock
    );
}

#[test]
fn distinct_commits_produce_their_own_declarations() {
    let (repository, first) = locked_repository("1.2.3");
    repository.write("flake.nix", valid_flake("2.0.0"));
    let second = repository.commit("🔖 Change declared version");

    let first_declaration = evaluate(&repository, "HEAD~1").unwrap();
    let second_declaration = evaluate(&repository, "HEAD").unwrap();

    assert_eq!(first_declaration.commit.to_string(), first);
    assert_eq!(first_declaration.declaration.version, Version::new(1, 2, 3));
    assert_eq!(second_declaration.commit.to_string(), second);
    assert_eq!(
        second_declaration.declaration.version,
        Version::new(2, 0, 0)
    );
}

#[test]
fn rejects_output_that_cannot_be_converted_to_json() {
    let repository = GitRepo::new();
    repository.write("flake.nix", flake("value: value"));
    repository.write("flake.lock", LOCK_FILE);
    repository.commit("🧪 Add invalid declaration output");

    let error = evaluate(&repository, "HEAD").unwrap_err();
    assert!(matches!(error, EvaluationError::Nix(_)), "{error}");
    assert!(
        error
            .to_string()
            .contains("convert presswerk.default to JSON")
    );
}

#[test]
fn rejects_unsupported_schemas_before_decoding_the_remainder() {
    let repository = GitRepo::new();
    repository.write(
        "flake.nix",
        flake("{ schemaVersion = 2; version = false; unexpected = true; }"),
    );
    repository.write("flake.lock", LOCK_FILE);
    repository.commit("🧪 Add future declaration");

    assert!(matches!(
        evaluate(&repository, "HEAD").unwrap_err(),
        EvaluationError::Declaration(DeclarationError::UnsupportedSchemaVersion(2))
    ));
}

#[test]
fn rejects_a_commit_without_a_lock_file_without_creating_one() {
    let repository = GitRepo::new();
    repository.write("flake.nix", valid_flake("1.2.3"));
    let commit = repository.commit("🧪 Omit flake lock");

    assert!(matches!(
        evaluate(&repository, "HEAD").unwrap_err(),
        EvaluationError::MissingLockFile(missing) if missing.to_string() == commit
    ));
    assert!(!repository.path().join("flake.lock").exists());
    assert_eq!(repository.git(["status", "--porcelain"]), "");
}

#[test]
fn rejects_a_stale_lock_file_without_modifying_it() {
    let (repository, _) = locked_repository("1.2.3");
    repository.write(
        "flake.nix",
        valid_flake("1.2.3").replace(
            "  outputs = { self }:",
            "  inputs.extra.url = \"path:./extra\";\n  outputs = { self, extra }:",
        ),
    );
    repository.write("extra/flake.nix", "{ outputs = { self }: {}; }\n");
    repository.commit("🧪 Make flake lock stale");
    let lock_before = fs::read(repository.path().join("flake.lock")).unwrap();

    let error = evaluate(&repository, "HEAD").unwrap_err();
    assert!(
        matches!(error, EvaluationError::LockFileRequiresChanges(_)),
        "{error}"
    );
    assert_eq!(
        fs::read(repository.path().join("flake.lock")).unwrap(),
        lock_before
    );
    assert_eq!(repository.git(["status", "--porcelain"]), "");
}
