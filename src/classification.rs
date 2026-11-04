use crate::declaration::{Tag, TagError};
use semver::Version;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::process::{Command, Output};

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
    repository: &Path,
    target: &str,
    tag: &Tag,
    mut version_at: F,
) -> Result<CommitClassification, ClassificationError>
where
    F: FnMut(&str) -> Result<Version, E>,
    E: Error + Send + Sync + 'static,
{
    let commit = resolve_commit(repository, target)?;
    let version = read_version(&commit, &mut version_at)?;
    let tag_name = tag.render(&version)?;
    let tagged_commit = tagged_commit(repository, &tag_name)?;

    if let Some(tagged_commit) = tagged_commit
        .as_ref()
        .filter(|tagged| tagged.as_str() != commit)
    {
        return if is_ancestor(repository, tagged_commit, &commit)? {
            Ok(CommitClassification::OrdinaryCommit)
        } else {
            Ok(CommitClassification::Conflict {
                tag: tag_name,
                tagged_commit: tagged_commit.clone(),
            })
        };
    }

    if tagged_commit.is_none()
        && version == Version::new(0, 0, 0)
        && !has_release_tags(repository, tag)?
    {
        return Ok(CommitClassification::OrdinaryCommit);
    }

    let parent = first_parent(repository, &commit)?;
    let parent_version = parent
        .as_deref()
        .map(|parent| read_version(parent, &mut version_at))
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
        repository,
        &commit,
        parent.zip(parent_version),
        &version,
        &mut version_at,
    )?;
    Ok(CommitClassification::PendingRelease {
        version,
        commit: pending_commit,
    })
}

fn find_pending_commit<F, E>(
    repository: &Path,
    commit: &str,
    mut parent: Option<(String, Version)>,
    version: &Version,
    version_at: &mut F,
) -> Result<String, ClassificationError>
where
    F: FnMut(&str) -> Result<Version, E>,
    E: Error + Send + Sync + 'static,
{
    let mut pending = commit.to_owned();

    while let Some((parent_commit, parent_version)) = parent {
        if parent_version != *version {
            return Ok(pending);
        }

        pending = parent_commit;
        parent = if let Some(parent) = first_parent(repository, &pending)? {
            let version = read_version(&parent, version_at)?;
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

fn resolve_commit(repository: &Path, revision: &str) -> Result<String, ClassificationError> {
    git_stdout(
        repository,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )
}

fn first_parent(repository: &Path, commit: &str) -> Result<Option<String>, ClassificationError> {
    let line = git_stdout(
        repository,
        &["rev-list", "--parents", "--max-count=1", commit],
    )?;
    Ok(line.split_whitespace().nth(1).map(str::to_owned))
}

fn tagged_commit(repository: &Path, tag_name: &str) -> Result<Option<String>, ClassificationError> {
    let reference = format!("refs/tags/{tag_name}");
    let exists = git(repository, &["show-ref", "--verify", "--quiet", &reference])?;
    match exists.status.code() {
        Some(0) => git_stdout(
            repository,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )
        .map(Some),
        Some(1) => Ok(None),
        _ => Err(git_failure(
            &["show-ref", "--verify", "--quiet", &reference],
            exists,
        )),
    }
}

fn is_ancestor(
    repository: &Path,
    ancestor: &str,
    commit: &str,
) -> Result<bool, ClassificationError> {
    let output = git(
        repository,
        &["merge-base", "--is-ancestor", ancestor, commit],
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(git_failure(
            &["merge-base", "--is-ancestor", ancestor, commit],
            output,
        )),
    }
}

fn has_release_tags(repository: &Path, tag: &Tag) -> Result<bool, ClassificationError> {
    let tags = git_stdout(
        repository,
        &["for-each-ref", "--format=%(refname:strip=2)", "refs/tags"],
    )?;
    for name in tags.lines() {
        if tag.parse(name)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_stdout(repository: &Path, arguments: &[&str]) -> Result<String, ClassificationError> {
    let output = git(repository, arguments)?;
    if !output.status.success() {
        return Err(git_failure(arguments, output));
    }

    String::from_utf8(output.stdout)
        .map(|stdout| stdout.trim().to_owned())
        .map_err(ClassificationError::GitOutput)
}

fn git(repository: &Path, arguments: &[&str]) -> Result<Output, ClassificationError> {
    Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .map_err(ClassificationError::RunGit)
}

fn git_failure(arguments: &[&str], output: Output) -> ClassificationError {
    ClassificationError::Git {
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

#[derive(Debug)]
pub enum ClassificationError {
    RunGit(std::io::Error),
    Git {
        arguments: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },
    GitOutput(std::string::FromUtf8Error),
    Tag(TagError),
    Version {
        commit: String,
        source: Box<dyn Error + Send + Sync>,
    },
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunGit(error) => write!(formatter, "could not run Git: {error}"),
            Self::Git {
                arguments,
                status,
                stderr,
            } => write!(
                formatter,
                "git {} failed with status {}: {stderr}",
                arguments.join(" "),
                status.map_or_else(|| "signal".into(), |status| status.to_string())
            ),
            Self::GitOutput(error) => write!(formatter, "Git returned non-UTF-8 output: {error}"),
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
            Self::RunGit(error) => Some(error),
            Self::GitOutput(error) => Some(error),
            Self::Tag(error) => Some(error),
            Self::Version { source, .. } => Some(source.as_ref()),
            Self::Git { .. } => None,
        }
    }
}

impl From<TagError> for ClassificationError {
    fn from(error: TagError) -> Self {
        Self::Tag(error)
    }
}
