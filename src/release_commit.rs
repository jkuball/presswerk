use git2::{ErrorCode, ObjectType, Oid, Repository};
use semver::Version;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};

const TRUSTED_PREDECESSOR_TRAILER: &str = "Presswerk-Trusted-Predecessor:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPredecessor {
    pub tag: String,
    pub commit: Oid,
}

impl TrustedPredecessor {
    pub fn from_argument(value: &str) -> Result<Self, TrustedPredecessorError> {
        let (tag, commit) = value.split_once('=').ok_or_else(|| {
            TrustedPredecessorError::new("trusted predecessor must use <tag>=<sha> form")
        })?;
        Self::parse(tag, commit)
    }

    pub fn from_commit_message(message: &str) -> Result<Option<Self>, TrustedPredecessorError> {
        let lines = message.lines().collect::<Vec<_>>();
        let trailers = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                line.strip_prefix(TRUSTED_PREDECESSOR_TRAILER)
                    .map(|value| (index, value))
            })
            .collect::<Vec<_>>();
        let Some((index, value)) = trailers.first().copied() else {
            return Ok(None);
        };
        if trailers.len() > 1 {
            return Err(TrustedPredecessorError::new(
                "release commit contains more than one trusted predecessor trailer",
            ));
        }
        if lines[index + 1..].iter().any(|line| !line.is_empty()) {
            return Err(TrustedPredecessorError::new(
                "trusted predecessor must be the final commit trailer",
            ));
        }
        let mut fields = value.split_whitespace();
        let tag = fields.next().unwrap_or_default();
        let commit = fields.next().unwrap_or_default();
        if fields.next().is_some() {
            return Err(TrustedPredecessorError::new(
                "trusted predecessor trailer must contain exactly <tag> <sha>",
            ));
        }
        Self::parse(tag, commit).map(Some)
    }

    fn parse(tag: &str, commit: &str) -> Result<Self, TrustedPredecessorError> {
        if tag.is_empty() || tag.chars().any(char::is_whitespace) {
            return Err(TrustedPredecessorError::new(
                "trusted predecessor tag must be non-empty and contain no whitespace",
            ));
        }
        let commit = Oid::from_str(commit).map_err(|error| {
            TrustedPredecessorError::new(format!(
                "trusted predecessor commit must be a full Git object ID: {error}"
            ))
        })?;
        Ok(Self {
            tag: tag.into(),
            commit,
        })
    }

    fn trailer(&self) -> String {
        format!("{TRUSTED_PREDECESSOR_TRAILER} {} {}", self.tag, self.commit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPredecessorError {
    message: String,
}

impl TrustedPredecessorError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TrustedPredecessorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for TrustedPredecessorError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

impl ReleaseFile {
    pub fn new(path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            contents: contents.into(),
        }
    }
}

pub fn create_release_commit(
    repository_path: impl AsRef<Path>,
    branch: &str,
    expected_parent: Oid,
    version: &Version,
    files: &[ReleaseFile],
    trusted_predecessor: Option<&TrustedPredecessor>,
) -> Result<Oid, ReleaseCommitError> {
    let repository = Repository::open(repository_path)
        .map_err(|source| ReleaseCommitError::git("could not open Git repository", source))?;
    let parent = repository.find_commit(expected_parent).map_err(|source| {
        ReleaseCommitError::git(
            format!("could not find parent commit {expected_parent}"),
            source,
        )
    })?;
    let mut tree_id = parent.tree_id();
    let mut paths = HashSet::new();

    for file in files {
        let components = path_components(&file.path)?;
        if !paths.insert(file.path.clone()) {
            return Err(ReleaseCommitError::InvalidPath {
                path: file.path.clone(),
                reason: "path is changed more than once",
            });
        }
        let blob = repository.blob(&file.contents).map_err(|source| {
            ReleaseCommitError::git(
                format!("could not write blob for {}", file.path.display()),
                source,
            )
        })?;
        tree_id = update_tree(&repository, Some(tree_id), &components, blob, &file.path)?;
    }

    let tree = repository.find_tree(tree_id).map_err(|source| {
        ReleaseCommitError::git(format!("could not read release tree {tree_id}"), source)
    })?;
    let signature = repository.signature().map_err(|source| {
        ReleaseCommitError::git("could not read Git author configuration", source)
    })?;
    let subject = format!("🔖 Release v{version}");
    let message = trusted_predecessor.map_or_else(
        || subject.clone(),
        |trusted| format!("{subject}\n\n{}", trusted.trailer()),
    );
    let commit = repository
        .commit(None, &signature, &signature, &message, &tree, &[&parent])
        .map_err(|source| ReleaseCommitError::git("could not create release commit", source))?;

    match repository.reference_matching(branch, commit, true, expected_parent, &subject) {
        Ok(_) => Ok(commit),
        Err(source) if matches!(source.code(), ErrorCode::Modified | ErrorCode::NotFound) => {
            let actual = repository
                .find_reference(branch)
                .ok()
                .and_then(|reference| reference.target());
            Err(ReleaseCommitError::BranchMoved {
                branch: branch.into(),
                expected: expected_parent,
                actual,
                commit,
            })
        }
        Err(source) => Err(ReleaseCommitError::git(
            format!("could not move {branch} to release commit {commit}"),
            source,
        )),
    }
}

fn path_components(path: &Path) -> Result<Vec<&str>, ReleaseCommitError> {
    let invalid = |reason| ReleaseCommitError::InvalidPath {
        path: path.to_owned(),
        reason,
    };
    if path.as_os_str().is_empty() || path.to_string_lossy().contains('\\') {
        return Err(invalid("path must be repository-relative"));
    }

    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(invalid("path must be repository-relative"));
        };
        let component = component
            .to_str()
            .ok_or_else(|| invalid("path must be valid UTF-8"))?;
        if component == ".git" {
            return Err(invalid("path must not enter .git"));
        }
        components.push(component);
    }
    if components.is_empty() {
        return Err(invalid("path must name a file"));
    }
    Ok(components)
}

fn update_tree(
    repository: &Repository,
    base: Option<Oid>,
    components: &[&str],
    blob: Oid,
    path: &Path,
) -> Result<Oid, ReleaseCommitError> {
    let base_tree = base
        .map(|id| repository.find_tree(id))
        .transpose()
        .map_err(|source| ReleaseCommitError::git("could not read base tree", source))?;
    let mut builder = repository
        .treebuilder(base_tree.as_ref())
        .map_err(|source| ReleaseCommitError::git("could not edit release tree", source))?;
    let name = components[0];

    if components.len() == 1 {
        let mode = match base_tree.as_ref().and_then(|tree| tree.get_name(name)) {
            Some(entry) if entry.kind() == Some(ObjectType::Blob) => entry.filemode(),
            Some(_) => {
                return Err(ReleaseCommitError::InvalidPath {
                    path: path.to_owned(),
                    reason: "path names a directory",
                });
            }
            None => 0o100644,
        };
        builder.insert(name, blob, mode).map_err(|source| {
            ReleaseCommitError::git(format!("could not update {}", path.display()), source)
        })?;
    } else {
        let subtree = match base_tree.as_ref().and_then(|tree| tree.get_name(name)) {
            Some(entry) if entry.kind() == Some(ObjectType::Tree) => Some(entry.id()),
            Some(_) => {
                return Err(ReleaseCommitError::InvalidPath {
                    path: path.to_owned(),
                    reason: "a parent component is not a directory",
                });
            }
            None => None,
        };
        let subtree = update_tree(repository, subtree, &components[1..], blob, path)?;
        builder.insert(name, subtree, 0o040000).map_err(|source| {
            ReleaseCommitError::git(format!("could not update {}", path.display()), source)
        })?;
    }

    builder
        .write()
        .map_err(|source| ReleaseCommitError::git("could not write release tree", source))
}

#[derive(Debug)]
pub enum ReleaseCommitError {
    BranchMoved {
        branch: String,
        expected: Oid,
        actual: Option<Oid>,
        commit: Oid,
    },
    InvalidPath {
        path: PathBuf,
        reason: &'static str,
    },
    Git {
        operation: String,
        source: git2::Error,
    },
}

impl ReleaseCommitError {
    fn git(operation: impl Into<String>, source: git2::Error) -> Self {
        Self::Git {
            operation: operation.into(),
            source,
        }
    }
}

impl fmt::Display for ReleaseCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BranchMoved {
                branch,
                expected,
                actual,
                commit,
            } => write!(
                formatter,
                "refused to move {branch} to release commit {commit}: expected {expected}, found {}",
                actual.map_or_else(|| "no commit".into(), |actual| actual.to_string())
            ),
            Self::InvalidPath { path, reason } => {
                write!(
                    formatter,
                    "invalid release path {}: {reason}",
                    path.display()
                )
            }
            Self::Git { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl Error for ReleaseCommitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git { source, .. } => Some(source),
            Self::BranchMoved { .. } | Self::InvalidPath { .. } => None,
        }
    }
}
