use crate::declaration::{DeclarationError, ReleaseDeclaration};
use anyhow::{Context as _, anyhow};
use git2::{ErrorCode, ObjectType, Oid, Repository};
use serde::Deserialize;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub struct EvaluatedReleaseDeclaration {
    pub commit: Oid,
    pub declaration: ReleaseDeclaration,
}

pub fn evaluate_release_declaration(
    repository_path: impl AsRef<Path>,
    revision: &str,
) -> Result<EvaluatedReleaseDeclaration, EvaluationError> {
    ReleaseDeclarationEvaluator::new().evaluate(repository_path, revision)
}

#[derive(Debug, Default)]
pub struct ReleaseDeclarationEvaluator {
    store_url: Option<String>,
}

impl ReleaseDeclarationEvaluator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_store_url(store_url: impl Into<String>) -> Self {
        Self {
            store_url: Some(store_url.into()),
        }
    }

    pub fn evaluate(
        &self,
        repository_path: impl AsRef<Path>,
        revision: &str,
    ) -> Result<EvaluatedReleaseDeclaration, EvaluationError> {
        let repository = Repository::open(repository_path).map_err(EvaluationError::Git)?;
        let commit = repository
            .revparse_single(revision)
            .and_then(|object| object.peel_to_commit())
            .map_err(EvaluationError::Git)?;
        let commit_id = commit.id();
        let tree = commit.tree().map_err(EvaluationError::Git)?;

        match tree.get_path(Path::new("flake.lock")) {
            Ok(entry) if entry.kind() == Some(ObjectType::Blob) => {}
            Ok(_) => return Err(EvaluationError::MissingLockFile(commit_id)),
            Err(error) if error.code() == ErrorCode::NotFound => {
                return Err(EvaluationError::MissingLockFile(commit_id));
            }
            Err(error) => return Err(EvaluationError::Git(error)),
        }

        let workdir = repository
            .workdir()
            .ok_or(EvaluationError::BareRepository)?
            .canonicalize()
            .map_err(EvaluationError::Io)?;
        let mut url = Url::from_directory_path(&workdir)
            .map_err(|()| EvaluationError::InvalidRepositoryPath(workdir.clone()))?;
        url.query_pairs_mut()
            .append_pair("rev", &commit_id.to_string());
        let flake_reference = format!("git+{url}");

        evaluate_pinned_flake(&flake_reference, commit_id, self.store_url.as_deref()).map(
            |declaration| EvaluatedReleaseDeclaration {
                commit: commit_id,
                declaration,
            },
        )
    }
}

#[derive(Deserialize)]
struct NixEvaluation {
    declaration: serde_json::Value,
    revision: Option<String>,
}

fn evaluate_pinned_flake(
    reference: &str,
    expected_commit: Oid,
    store_url: Option<&str>,
) -> Result<ReleaseDeclaration, EvaluationError> {
    let reference = serde_json::to_string(reference).expect("flake reference is a JSON string");
    // A virtual lock update omits sourceInfo.rev, so return it beside the declaration.
    let expression = format!(
        "let flake = builtins.getFlake {reference}; in {{ declaration = flake.presswerk.default; revision = flake.sourceInfo.rev or null; }}"
    );
    let mut command = Command::new("nix");
    command.args([
        "eval",
        "--json",
        "--no-update-lock-file",
        "--no-write-lock-file",
    ]);
    if let Some(store_url) = store_url {
        command.args(["--store", store_url]);
    }
    let output = command
        .args(["--expr", &expression])
        .output()
        .context("run nix eval")
        .map_err(EvaluationError::Nix)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim_end();
        let message = if stderr.is_empty() {
            format!("nix eval failed with {}", output.status)
        } else {
            format!("nix eval failed with {}:\n{stderr}", output.status)
        };
        return Err(EvaluationError::Nix(anyhow!(message)));
    }

    let evaluated: NixEvaluation = serde_json::from_slice(&output.stdout)
        .context("decode nix eval output")
        .map_err(EvaluationError::Nix)?;
    let Some(locked_revision) = evaluated.revision else {
        return Err(EvaluationError::LockFileRequiresChanges(expected_commit));
    };
    if locked_revision != expected_commit.to_string() {
        return Err(EvaluationError::UnexpectedLockedRevision {
            expected: expected_commit,
            actual: locked_revision,
        });
    }

    let json = serde_json::to_string(&evaluated.declaration)
        .context("encode evaluated release declaration")
        .map_err(EvaluationError::Nix)?;
    ReleaseDeclaration::from_json(&json).map_err(EvaluationError::Declaration)
}

#[derive(Debug)]
pub enum EvaluationError {
    Git(git2::Error),
    Io(std::io::Error),
    BareRepository,
    InvalidRepositoryPath(PathBuf),
    MissingLockFile(Oid),
    LockFileRequiresChanges(Oid),
    UnexpectedLockedRevision { expected: Oid, actual: String },
    Nix(anyhow::Error),
    Declaration(DeclarationError),
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => write!(formatter, "resolve Git revision: {error}"),
            Self::Io(error) => write!(formatter, "resolve repository path: {error}"),
            Self::BareRepository => formatter.write_str("cannot evaluate a bare Git repository"),
            Self::InvalidRepositoryPath(path) => {
                write!(
                    formatter,
                    "repository path cannot be represented as a file URL: {path:?}"
                )
            }
            Self::MissingLockFile(commit) => {
                write!(formatter, "commit {commit} does not contain flake.lock")
            }
            Self::LockFileRequiresChanges(commit) => {
                write!(formatter, "flake.lock at commit {commit} requires changes")
            }
            Self::UnexpectedLockedRevision { expected, actual } => write!(
                formatter,
                "pinned flake resolved to revision {actual} instead of {expected}"
            ),
            Self::Nix(error) => write!(formatter, "evaluate release declaration: {error:#}"),
            Self::Declaration(error) => error.fmt(formatter),
        }
    }
}

impl Error for EvaluationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Nix(error) => Some(error.as_ref()),
            Self::Declaration(error) => Some(error),
            _ => None,
        }
    }
}
