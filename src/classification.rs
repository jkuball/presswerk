use crate::declaration::{Tag, TagError};
use git2::{ErrorCode, Oid, Repository};
use semver::Version;
use std::error::Error;
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitClassification {
    OrdinaryCommit,
    NewRelease,
    PendingRelease { version: Version, commit: String },
    RetryOrRepublish,
    InvalidCandidate,
    Conflict { tag: String, tagged_commit: String },
}

impl fmt::Display for CommitClassification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrdinaryCommit => formatter.write_str("ordinary commit"),
            Self::NewRelease => formatter.write_str("new release"),
            Self::PendingRelease { version, commit } => write!(
                formatter,
                "pending release {version} at {commit} is not published yet"
            ),
            Self::RetryOrRepublish => formatter.write_str("retry or republish"),
            Self::InvalidCandidate => formatter.write_str("invalid candidate"),
            Self::Conflict { tag, tagged_commit } => {
                write!(
                    formatter,
                    "tag {tag} points to non-ancestor {tagged_commit}"
                )
            }
        }
    }
}

pub fn classify_commit<F, E>(
    repository_path: &Path,
    target: &str,
    tag: &Tag,
    mut version_at: F,
) -> Result<CommitClassification, ClassificationError>
where
    F: FnMut(&str) -> Result<Version, E>,
    E: Error + Send + Sync + 'static,
{
    let repository = Repository::open(repository_path).map_err(|source| {
        ClassificationError::git(
            format!(
                "could not open Git repository {}",
                repository_path.display()
            ),
            source,
        )
    })?;
    let commit = resolve_commit(&repository, target)?;
    let commit_id = commit.to_string();
    let version = read_version(&commit_id, &mut version_at)?;
    let tag_name = tag.render(&version)?;
    let tagged_commit = tagged_commit(&repository, &tag_name)?;

    if let Some(tagged_commit) = tagged_commit.filter(|tagged| *tagged != commit) {
        return if is_ancestor(&repository, tagged_commit, commit)? {
            Ok(CommitClassification::OrdinaryCommit)
        } else {
            Ok(CommitClassification::Conflict {
                tag: tag_name,
                tagged_commit: tagged_commit.to_string(),
            })
        };
    }

    if tagged_commit.is_none()
        && version == Version::new(0, 0, 0)
        && !has_release_tags(&repository, tag)?
    {
        return Ok(CommitClassification::OrdinaryCommit);
    }

    let parent = first_parent(&repository, commit)?;
    let parent_version = parent
        .map(|parent| read_version(&parent.to_string(), &mut version_at))
        .transpose()?;
    let version_changed = parent_version
        .as_ref()
        .is_none_or(|parent_version| *parent_version != version);

    if tagged_commit.is_some() {
        return Ok(if version_changed {
            CommitClassification::RetryOrRepublish
        } else {
            CommitClassification::InvalidCandidate
        });
    }

    if version_changed {
        return Ok(CommitClassification::NewRelease);
    }

    let pending_commit = find_pending_commit(
        &repository,
        commit,
        parent.zip(parent_version),
        &version,
        &mut version_at,
    )?;
    Ok(CommitClassification::PendingRelease {
        version,
        commit: pending_commit.to_string(),
    })
}

fn find_pending_commit<F, E>(
    repository: &Repository,
    commit: Oid,
    mut parent: Option<(Oid, Version)>,
    version: &Version,
    version_at: &mut F,
) -> Result<Oid, ClassificationError>
where
    F: FnMut(&str) -> Result<Version, E>,
    E: Error + Send + Sync + 'static,
{
    let mut pending = commit;

    while let Some((parent_commit, parent_version)) = parent {
        if parent_version != *version {
            return Ok(pending);
        }

        pending = parent_commit;
        parent = if let Some(parent) = first_parent(repository, pending)? {
            let version = read_version(&parent.to_string(), version_at)?;
            Some((parent, version))
        } else {
            None
        };
    }

    Ok(pending)
}

fn read_version<F, E>(commit: &str, version_at: &mut F) -> Result<Version, ClassificationError>
where
    F: FnMut(&str) -> Result<Version, E>,
    E: Error + Send + Sync + 'static,
{
    version_at(commit).map_err(|source| ClassificationError::Version {
        commit: commit.to_owned(),
        source: Box::new(source),
    })
}

fn resolve_commit(repository: &Repository, revision: &str) -> Result<Oid, ClassificationError> {
    let object = repository.revparse_single(revision).map_err(|source| {
        ClassificationError::git(
            format!("could not resolve Git revision {revision:?}"),
            source,
        )
    })?;
    object
        .peel_to_commit()
        .map(|commit| commit.id())
        .map_err(|source| {
            ClassificationError::git(
                format!("Git revision {revision:?} does not resolve to a commit"),
                source,
            )
        })
}

fn first_parent(repository: &Repository, commit: Oid) -> Result<Option<Oid>, ClassificationError> {
    let commit = repository.find_commit(commit).map_err(|source| {
        ClassificationError::git(format!("could not read Git commit {commit}"), source)
    })?;
    if commit.parent_count() == 0 {
        Ok(None)
    } else {
        commit.parent_id(0).map(Some).map_err(|source| {
            ClassificationError::git(
                format!("could not read first parent of Git commit {}", commit.id()),
                source,
            )
        })
    }
}

fn tagged_commit(
    repository: &Repository,
    tag_name: &str,
) -> Result<Option<Oid>, ClassificationError> {
    let reference_name = format!("refs/tags/{tag_name}");
    let reference = match repository.find_reference(&reference_name) {
        Ok(reference) => reference,
        Err(error) if error.code() == ErrorCode::NotFound => return Ok(None),
        Err(source) => {
            return Err(ClassificationError::git(
                format!("could not read Git tag {tag_name:?}"),
                source,
            ));
        }
    };

    reference
        .peel_to_commit()
        .map(|commit| Some(commit.id()))
        .map_err(|source| {
            ClassificationError::git(
                format!("Git tag {tag_name:?} does not resolve to a commit"),
                source,
            )
        })
}

fn is_ancestor(
    repository: &Repository,
    ancestor: Oid,
    commit: Oid,
) -> Result<bool, ClassificationError> {
    repository
        .graph_descendant_of(commit, ancestor)
        .map_err(|source| {
            ClassificationError::git(
                format!("could not check whether {ancestor} is an ancestor of {commit}"),
                source,
            )
        })
}

fn has_release_tags(repository: &Repository, tag: &Tag) -> Result<bool, ClassificationError> {
    let tags = repository
        .tag_names(None)
        .map_err(|source| ClassificationError::git("could not list Git tags", source))?;
    for name in tags.iter() {
        let name = name.map_err(|source| {
            ClassificationError::git("could not decode a Git tag name", source)
        })?;
        let Some(name) = name else { continue };
        if tag.parse(name)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug)]
pub enum ClassificationError {
    Git {
        operation: String,
        source: git2::Error,
    },
    Tag(TagError),
    Version {
        commit: String,
        source: Box<dyn Error + Send + Sync>,
    },
}

impl ClassificationError {
    fn git(operation: impl Into<String>, source: git2::Error) -> Self {
        Self::Git {
            operation: operation.into(),
            source,
        }
    }
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::Tag(error) => error.fmt(formatter),
            Self::Version { commit, source } => {
                write!(formatter, "could not read version at {commit}: {source}")
            }
        }
    }
}

impl Error for ClassificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git { source, .. } => Some(source),
            Self::Tag(error) => Some(error),
            Self::Version { source, .. } => Some(source.as_ref()),
        }
    }
}

impl From<TagError> for ClassificationError {
    fn from(error: TagError) -> Self {
        Self::Tag(error)
    }
}
