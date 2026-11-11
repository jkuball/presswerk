use crate::declaration::{DeclarationError, ReleaseDeclaration};
use anyhow::Context as _;
use git2::{ErrorCode, ObjectType, Oid, Repository};
use nix_bindings_expr::eval_state::{EvalStateBuilder, gc_register_my_thread};
use nix_bindings_fetchers::FetchersSettings;
use nix_bindings_flake::{
    EvalStateBuilderExt, FlakeLockFlags, FlakeReference, FlakeReferenceParseFlags, FlakeSettings,
    LockedFlake,
};
use nix_bindings_store::store::Store;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use url::Url;

pub(crate) static NIX_EVALUATION: Mutex<()> = Mutex::new(());

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

        let _evaluation = NIX_EVALUATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        evaluate_pinned_flake(&flake_reference, commit_id, self.store_url.as_deref()).map(
            |declaration| EvaluatedReleaseDeclaration {
                commit: commit_id,
                declaration,
            },
        )
    }
}

fn evaluate_pinned_flake(
    reference: &str,
    expected_commit: Oid,
    store_url: Option<&str>,
) -> Result<ReleaseDeclaration, EvaluationError> {
    nix_bindings_expr::eval_state::init()
        .context("initialize the Nix expression evaluator")
        .map_err(EvaluationError::Nix)?;
    let experimental_features = nix_bindings_util::settings::get("experimental-features")
        .context("read Nix experimental features")
        .map_err(EvaluationError::Nix)?;
    if !experimental_features
        .split_whitespace()
        .any(|feature| feature == "flakes")
    {
        nix_bindings_util::settings::set(
            "experimental-features",
            &format!("{experimental_features} flakes"),
        )
        .context("enable Nix flakes")
        .map_err(EvaluationError::Nix)?;
    }

    let _gc_registration = gc_register_my_thread()
        .context("register the evaluation thread with the Nix garbage collector")
        .map_err(EvaluationError::Nix)?;
    let store = Store::open(store_url, [])
        .context("open the Nix store")
        .map_err(EvaluationError::Nix)?;
    let fetchers_settings = FetchersSettings::new()
        .context("create Nix fetcher settings")
        .map_err(EvaluationError::Nix)?;
    let flake_settings = FlakeSettings::new()
        .context("create Nix flake settings")
        .map_err(EvaluationError::Nix)?;
    let mut eval_state = EvalStateBuilder::new(store)
        .and_then(|builder| builder.flakes(&flake_settings))
        .and_then(|builder| builder.build())
        .context("create the Nix evaluation state")
        .map_err(EvaluationError::Nix)?;

    let parse_flags = FlakeReferenceParseFlags::new(&flake_settings)
        .context("create flake reference parse flags")
        .map_err(EvaluationError::Nix)?;
    let (flake_reference, fragment) = FlakeReference::parse_with_fragment(
        &fetchers_settings,
        &flake_settings,
        &parse_flags,
        reference,
    )
    .context("parse the pinned flake reference")
    .map_err(EvaluationError::Nix)?;
    if !fragment.is_empty() {
        return Err(EvaluationError::UnexpectedFragment(fragment));
    }

    let mut lock_flags = FlakeLockFlags::new(&flake_settings)
        .context("create flake lock flags")
        .map_err(EvaluationError::Nix)?;
    lock_flags
        .set_mode_check()
        .context("configure check-only flake locking")
        .map_err(EvaluationError::Nix)?;
    let locked_flake = LockedFlake::lock(
        &fetchers_settings,
        &flake_settings,
        &eval_state,
        &lock_flags,
        &flake_reference,
    )
    .context("check the committed flake lock")
    .map_err(EvaluationError::Nix)?;

    let outputs = locked_flake
        .outputs(&flake_settings, &mut eval_state)
        .context("evaluate flake outputs")
        .map_err(EvaluationError::Nix)?;
    // Nix marks a virtually updated lock dirty by omitting the pinned root revision.
    let source_info = eval_state
        .require_attrs_select(&outputs, "sourceInfo")
        .context("read locked flake source information")
        .map_err(EvaluationError::Nix)?;
    let locked_revision = eval_state
        .require_attrs_select_opt(&source_info, "rev")
        .context("read locked flake revision")
        .map_err(EvaluationError::Nix)?
        .ok_or(EvaluationError::LockFileRequiresChanges(expected_commit))?;
    let locked_revision = eval_state
        .require_string(&locked_revision)
        .context("read locked flake revision as a string")
        .map_err(EvaluationError::Nix)?;
    if locked_revision != expected_commit.to_string() {
        return Err(EvaluationError::UnexpectedLockedRevision {
            expected: expected_commit,
            actual: locked_revision,
        });
    }

    let presswerk = eval_state
        .require_attrs_select(&outputs, "presswerk")
        .context("select flake output presswerk")
        .map_err(EvaluationError::Nix)?;
    let declaration = eval_state
        .require_attrs_select(&presswerk, "default")
        .context("select flake output presswerk.default")
        .map_err(EvaluationError::Nix)?;
    let to_json = eval_state
        .eval_from_string("builtins.toJSON", "<presswerk>")
        .context("load builtins.toJSON")
        .map_err(EvaluationError::Nix)?;
    let json = eval_state
        .call(to_json, declaration)
        .and_then(|value| eval_state.require_string(&value))
        .context("convert presswerk.default to JSON")
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
    UnexpectedFragment(String),
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
            Self::UnexpectedFragment(fragment) => {
                write!(
                    formatter,
                    "pinned flake reference unexpectedly contains fragment {fragment:?}"
                )
            }
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
