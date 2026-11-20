use crate::analysis::{Analysis, Analyzer, CommitRange, GitCliffAnalyzer};
use crate::changelog::{ChangelogHeading, read_version_section};
use crate::classification::{CommitClassification, classify_commit};
use crate::declaration::{
    DeclarationError, GithubReleasePublisher as GithubReleaseDeclaration,
    OciHelmPublisher as OciHelmDeclaration, OciImagePublisher as OciImageDeclaration,
    ReleaseDeclaration, UNSTABLE_SCHEMA_VERSION,
};
use crate::evaluation::{EvaluationError, evaluate_release_declaration};
use crate::publishers::github_release::{
    GithubClient, GithubPublisher, GithubReleaseReader, GithubReleaseState,
};
use crate::release_commit::TrustedPredecessor;
use git2::{ErrorCode, ObjectType, Oid, Repository, Signature};
use semver::Version;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;

/// Why a predecessor's declaration cannot be read, so only `--trust-predecessor` can cross it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadableDeclaration {
    /// Written for a schema version this CLI cannot read.
    UnsupportedSchema(i64),
    /// The predecessor has no Presswerk declaration, e.g. it was released by another tool.
    Missing,
}

/// A predecessor release whose declaration cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadablePredecessor {
    pub tag: String,
    pub commit: Oid,
    pub declaration: UnreadableDeclaration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationError {
    message: String,
    unreadable_predecessor: Option<UnreadablePredecessor>,
}

impl PublicationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unreadable_predecessor: None,
        }
    }

    pub fn unsupported_predecessor(
        tag: impl Into<String>,
        commit: Oid,
        schema_version: i64,
    ) -> Self {
        let tag = tag.into();
        let message = format!(
            "previous release tag {tag:?} at {commit} uses a declaration with schemaVersion {schema_version} that this CLI cannot read; verify that release with its original Presswerk toolchain, then run `presswerk prepare --trust-predecessor {tag}={commit}`"
        );
        Self::unreadable_predecessor(
            message,
            tag,
            commit,
            UnreadableDeclaration::UnsupportedSchema(schema_version),
        )
    }

    pub fn undeclared_predecessor(tag: impl Into<String>, commit: Oid) -> Self {
        let tag = tag.into();
        let message = format!(
            "previous release {tag} at {commit} has no Presswerk declaration\n  If {tag} was released completely by another tool, adopt it once:\n    presswerk prepare --trust-predecessor {tag}={commit}"
        );
        Self::unreadable_predecessor(message, tag, commit, UnreadableDeclaration::Missing)
    }

    fn unreadable_predecessor(
        message: String,
        tag: String,
        commit: Oid,
        declaration: UnreadableDeclaration,
    ) -> Self {
        Self {
            message,
            unreadable_predecessor: Some(UnreadablePredecessor {
                tag,
                commit,
                declaration,
            }),
        }
    }

    #[must_use]
    pub fn as_unreadable_predecessor(&self) -> Option<&UnreadablePredecessor> {
        self.unreadable_predecessor.as_ref()
    }
}

impl fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PublicationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub commit: String,
    pub version: Version,
    pub tag: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteState {
    Missing,
    Matching,
    Conflicting(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteTag {
    Annotated(Oid),
    Lightweight(Oid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherState {
    Missing,
    Draft,
    /// Published, but some of its artifacts are missing and can be added.
    Incomplete,
    Published,
    /// Published, but best-effort aliases such as an OCI `latest` tag still need to move. The
    /// release counts as complete; a publish moves them, and a failure there is only a warning.
    AliasesPending,
    Conflicting(String),
}

/// What happened to one artifact of a publisher, such as a GitHub Release asset or an OCI tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetStep {
    Uploaded,
    AlreadyUploaded,
    /// Deliberately left as it is, such as an alias that points to a higher version.
    Kept(String),
    /// An alias that the tool before Presswerk pushed for the trusted predecessor, moved to this
    /// release once.
    TakenOver,
    /// A best-effort step that failed, such as an alias that could not be moved; a rerun repairs it.
    Warning(String),
}

/// The kind of artifact a publisher reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A GitHub Release asset.
    Asset,
    /// An OCI image for one platform.
    Image,
    /// An OCI image or Helm chart tag that names the version.
    Tag,
    /// An OCI tag that follows the highest final version, such as `latest`.
    Alias,
}

/// One artifact of a publisher, with what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactStep {
    pub kind: ArtifactKind,
    pub name: String,
    pub step: AssetStep,
}

/// Each artifact of a publisher, with what happened to it.
pub type ArtifactSteps = Vec<ArtifactStep>;

/// One completed step of `publish`, reported as soon as it is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishEvent<'a> {
    /// Publishing starts; nothing has been written yet.
    Started { tag: &'a str, commit: &'a str },
    /// The Git tag is complete.
    Tag { tag: &'a str, step: TagStep },
    /// One artifact of the publisher that runs now is complete.
    Artifact(&'a ArtifactStep),
    /// A publisher is complete, after all of its artifacts.
    Publisher { name: &'a str, step: PublisherStep },
    /// A publisher failed after the tag was pushed.
    PublisherFailed { name: &'a str, reason: &'a str },
    /// A publisher did not run because a dependency failed or fail-fast stopped the graph.
    PublisherSkipped { name: &'a str, reason: &'a str },
}

/// Receives each step of `publish` as soon as it completes.
pub type Progress<'a> = dyn FnMut(PublishEvent<'_>) + Send + 'a;

pub trait TagPublisher {
    fn state(&self, release: &Release) -> Result<RemoteState, PublicationError>;
    fn publish(&self, release: &Release) -> Result<(), PublicationError>;
    /// The commit a lightweight remote tag points to, or `None` if the tag is not lightweight.
    fn lightweight_target(&self, _tag: &str) -> Result<Option<Oid>, PublicationError> {
        Ok(None)
    }
}

pub trait Publisher: Send + Sync {
    fn state(&self, release: &Release) -> Result<PublisherState, PublicationError>;
    /// Create every missing part and report each artifact, passing each one to `progress` as
    /// soon as it is complete.
    fn publish_with_progress(
        &self,
        release: &Release,
        progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError>;
    /// Create every missing part and report each artifact.
    fn publish(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        self.publish_with_progress(release, &mut |_| {})
    }
    /// Report the artifacts of a publisher whose state is already published, without writing.
    fn complete_steps(&self, _release: &Release) -> Result<ArtifactSteps, PublicationError> {
        Ok(Vec::new())
    }
}

/// One publisher in the declared dependency graph.
pub struct PublisherNode {
    pub name: String,
    pub after: Vec<String>,
    pub publisher: Box<dyn Publisher>,
}

impl PublisherNode {
    pub fn new(name: impl Into<String>, publisher: Box<dyn Publisher>, after: Vec<String>) -> Self {
        Self {
            name: name.into(),
            after,
            publisher,
        }
    }
}

#[derive(Clone)]
pub struct GitTagPublisher {
    repository: PathBuf,
    remote: String,
    tagger: Option<Signature<'static>>,
}

impl GitTagPublisher {
    pub fn new(repository: impl Into<PathBuf>, remote: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            remote: remote.into(),
            tagger: None,
        }
    }

    /// Reads the tagger identity up front, so a missing one stops `publish` before anything is
    /// written. The annotated tag reuses it.
    pub fn with_tagger(mut self) -> Result<Self, PublicationError> {
        let repository = self.open_repository()?;
        self.tagger = Some(read_tagger(&repository)?);
        Ok(self)
    }

    fn open_repository(&self) -> Result<Repository, PublicationError> {
        Repository::open(&self.repository).map_err(|error| {
            PublicationError::new(format!(
                "could not open Git repository {}: {error}",
                self.repository.display()
            ))
        })
    }

    fn remote_target(&self, tag: &str) -> Result<Option<RemoteTag>, PublicationError> {
        let reference = format!("refs/tags/{tag}");
        let peeled = format!("{reference}^{{}}");
        let output = Command::new("git")
            .args(["ls-remote", "--tags", &self.remote, &reference, &peeled])
            .current_dir(&self.repository)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| {
                PublicationError::new(format!("could not run git ls-remote: {error}"))
            })?;
        if !output.status.success() {
            return Err(PublicationError::new(format!(
                "could not inspect Git tag {tag:?} on remote {:?}: {}",
                self.remote,
                git_failure(&output)
            )));
        }

        let output = std::str::from_utf8(&output.stdout).map_err(|error| {
            PublicationError::new(format!(
                "remote {:?} returned non-UTF-8 tag state: {error}",
                self.remote
            ))
        })?;
        let mut direct = None;
        let mut dereferenced = None;
        for line in output.lines() {
            let (oid, name) = line.split_once('\t').ok_or_else(|| {
                PublicationError::new(format!(
                    "remote {:?} returned malformed tag state for {tag:?}",
                    self.remote
                ))
            })?;
            let oid = Oid::from_str(oid).map_err(|error| {
                PublicationError::new(format!(
                    "remote {:?} returned an invalid object ID for {tag:?}: {error}",
                    self.remote
                ))
            })?;
            if name == reference {
                direct = Some(oid);
            } else if name == peeled {
                dereferenced = Some(oid);
            } else {
                return Err(PublicationError::new(format!(
                    "remote {:?} returned unexpected reference {name:?} for {tag:?}",
                    self.remote
                )));
            }
        }
        match (direct, dereferenced) {
            (None, None) => Ok(None),
            (Some(direct), None) => Ok(Some(RemoteTag::Lightweight(direct))),
            (Some(_), Some(commit)) => Ok(Some(RemoteTag::Annotated(commit))),
            (None, Some(_)) => Err(PublicationError::new(format!(
                "remote {:?} returned a peeled tag without {reference}",
                self.remote
            ))),
        }
    }

    fn create_local_tag(&self, release: &Release) -> Result<(), PublicationError> {
        let repository = self.open_repository()?;
        let commit = Oid::from_str(&release.commit).map_err(|error| {
            PublicationError::new(format!(
                "invalid release commit {:?}: {error}",
                release.commit
            ))
        })?;
        let target = repository
            .find_object(commit, Some(ObjectType::Commit))
            .map_err(|error| {
                PublicationError::new(format!("could not read release commit {commit}: {error}"))
            })?;
        let reference = format!("refs/tags/{}", release.tag);
        match repository.find_reference(&reference) {
            Ok(existing) => {
                let tagged = existing.peel_to_commit().map_err(|error| {
                    PublicationError::new(format!(
                        "local Git tag {:?} does not point to a commit: {error}",
                        release.tag
                    ))
                })?;
                if tagged.id() == commit {
                    existing.peel(ObjectType::Tag).map_err(|_| {
                        PublicationError::new(format!(
                            "local Git tag {:?} is not annotated",
                            release.tag
                        ))
                    })?;
                    return Ok(());
                }
                return Err(PublicationError::new(format!(
                    "local Git tag {:?} points to {}, expected {commit}",
                    release.tag,
                    tagged.id()
                )));
            }
            Err(error) if error.code() == ErrorCode::NotFound => {}
            Err(error) => {
                return Err(PublicationError::new(format!(
                    "could not inspect local Git tag {:?}: {error}",
                    release.tag
                )));
            }
        }

        let signature = match &self.tagger {
            Some(tagger) => tagger.clone(),
            None => read_tagger(&repository)?,
        };
        repository
            .tag(
                &release.tag,
                &target,
                &signature,
                &format!("Release {}", release.version),
                false,
            )
            .map_err(|error| {
                PublicationError::new(format!(
                    "could not create annotated Git tag {:?}: {error}",
                    release.tag
                ))
            })?;
        Ok(())
    }
}

impl TagPublisher for GitTagPublisher {
    fn lightweight_target(&self, tag: &str) -> Result<Option<Oid>, PublicationError> {
        Ok(match self.remote_target(tag)? {
            Some(RemoteTag::Lightweight(commit)) => Some(commit),
            _ => None,
        })
    }

    fn state(&self, release: &Release) -> Result<RemoteState, PublicationError> {
        let expected = Oid::from_str(&release.commit).map_err(|error| {
            PublicationError::new(format!(
                "invalid release commit {:?}: {error}",
                release.commit
            ))
        })?;
        Ok(match self.remote_target(&release.tag)? {
            None => RemoteState::Missing,
            Some(RemoteTag::Annotated(actual)) if actual == expected => RemoteState::Matching,
            Some(RemoteTag::Annotated(actual)) => RemoteState::Conflicting(format!(
                "annotated Git tag {:?} on remote {:?} points to {actual}, expected {expected}",
                release.tag, self.remote
            )),
            Some(RemoteTag::Lightweight(actual)) => RemoteState::Conflicting(format!(
                "Git tag {:?} on remote {:?} is lightweight and points to {actual}",
                release.tag, self.remote
            )),
        })
    }

    fn publish(&self, release: &Release) -> Result<(), PublicationError> {
        match self.state(release)? {
            RemoteState::Matching => return Ok(()),
            RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
            RemoteState::Missing => {}
        }
        self.create_local_tag(release)?;

        let reference = format!("refs/tags/{0}:refs/tags/{0}", release.tag);
        let output = Command::new("git")
            .args(["push", "--porcelain", &self.remote, &reference])
            .current_dir(&self.repository)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| PublicationError::new(format!("could not run git push: {error}")))?;
        if output.status.success() {
            return match self.state(release)? {
                RemoteState::Matching => Ok(()),
                state => Err(PublicationError::new(format!(
                    "Git tag {:?} was not published: {state:?}",
                    release.tag
                ))),
            };
        }

        match self.state(release)? {
            RemoteState::Matching => Ok(()),
            RemoteState::Conflicting(message) => Err(PublicationError::new(message)),
            RemoteState::Missing => Err(PublicationError::new(format!(
                "could not push Git tag {:?} to remote {:?}: {}",
                release.tag,
                self.remote,
                git_failure(&output)
            ))),
        }
    }
}

fn read_tagger(repository: &Repository) -> Result<Signature<'static>, PublicationError> {
    repository.signature().map_err(|error| {
        PublicationError::new(format!(
            "could not read Git tagger configuration: {error}; annotated release tags need \
             `user.name` and `user.email`, so set them with `git config` in the job that runs \
             `publish`"
        ))
    })
}

/// Why a git command failed: its exit status, what it printed on stderr, and the refs a porcelain
/// push reports as rejected, which carry the remote's reason.
fn git_failure(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details: Vec<_> = stderr
        .lines()
        .chain(stdout.lines().filter(|line| line.starts_with('!')))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if details.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        format!(
            "git exited with {}:\n  {}",
            output.status,
            details.join("\n  ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    /// The commit is an ordinary commit, not a release commit.
    NothingToDo {
        commit: String,
    },
    Published(PublishReport),
}

/// What publication did, step by step; already complete steps show that a retry changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub commit: String,
    pub tag: String,
    pub tag_step: TagStep,
    /// One outcome per publisher, named like its declaration, such as `github-release.default`.
    pub publisher_outcomes: Vec<(String, PublisherOutcome)>,
}

impl PublishReport {
    /// The best-effort steps that failed, by artifact name, with their reason.
    #[must_use]
    pub fn warnings(&self) -> Vec<(&str, &str)> {
        self.publisher_outcomes
            .iter()
            .filter_map(|(_, outcome)| match outcome {
                PublisherOutcome::Published { artifacts, .. } => Some(artifacts),
                PublisherOutcome::Failed { .. } | PublisherOutcome::Skipped { .. } => None,
            })
            .flatten()
            .filter_map(|artifact| match &artifact.step {
                AssetStep::Warning(reason) => Some((artifact.name.as_str(), reason.as_str())),
                _ => None,
            })
            .collect()
    }

    /// Whether every step was already complete before this run.
    #[must_use]
    pub fn changed_nothing(&self) -> bool {
        self.tag_step == TagStep::AlreadyPushed
            && self.publisher_outcomes.iter().all(|(_, outcome)| {
                matches!(
                    outcome,
                    PublisherOutcome::Published {
                        step: PublisherStep::AlreadyPublished,
                        ..
                    }
                )
            })
    }

    #[must_use]
    pub fn failed(&self) -> usize {
        self.publisher_outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, PublisherOutcome::Failed { .. }))
            .count()
    }

    #[must_use]
    pub fn skipped(&self) -> usize {
        self.publisher_outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, PublisherOutcome::Skipped { .. }))
            .count()
    }

    #[must_use]
    pub fn complete(&self) -> bool {
        self.failed() == 0 && self.skipped() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherOutcome {
    Published {
        step: PublisherStep,
        artifacts: ArtifactSteps,
    },
    Failed {
        reason: String,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagStep {
    Pushed,
    AlreadyPushed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublisherStep {
    Published,
    DraftCompleted,
    /// The release was published before, and missing artifacts were added.
    ArtifactsCompleted,
    AlreadyPublished,
}

pub fn publish(
    repository_path: impl AsRef<Path>,
    revision: &str,
    jobs: NonZeroUsize,
    fail_fast: bool,
    progress: &mut Progress<'_>,
) -> Result<PublishOutcome, PublicationError> {
    publish_to_remote(
        repository_path.as_ref(),
        revision,
        "origin",
        &GitCliffAnalyzer,
        jobs,
        fail_fast,
        progress,
    )
}

pub fn publish_to_remote(
    repository_path: &Path,
    revision: &str,
    remote: &str,
    analyzer: &dyn Analyzer,
    jobs: NonZeroUsize,
    fail_fast: bool,
    progress: &mut Progress<'_>,
) -> Result<PublishOutcome, PublicationError> {
    let evaluated = evaluate_release_declaration(repository_path, revision)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    let commit = evaluated.commit;
    let declaration = evaluated.declaration;
    let commit_id = commit.to_string();
    let candidate_version = declaration.version.clone();
    let classification =
        classify_commit(repository_path, &commit_id, &declaration.tag, |revision| {
            if revision == commit_id {
                Ok(candidate_version.clone())
            } else {
                evaluate_release_declaration(repository_path, revision)
                    .map(|evaluated| evaluated.declaration.version)
            }
        })
        .map_err(|error| PublicationError::new(error.to_string()))?;

    match classification {
        CommitClassification::OrdinaryCommit => {
            return Ok(PublishOutcome::NothingToDo { commit: commit_id });
        }
        CommitClassification::NewRelease | CommitClassification::RetryOrRepublish => {}
        other => {
            return Err(PublicationError::new(format!(
                "cannot publish {commit}: {other}"
            )));
        }
    }

    validate_release_publisher_tags(&declaration)?;

    let tag = declaration
        .tag
        .render(&declaration.version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    let changelog = read_text_at(repository_path, commit, &declaration.changelog.path)?;
    let notes = declaration
        .changelog
        .heading()
        .and_then(|heading| read_version_section(&changelog, &heading, &declaration.version))
        .map_err(|error| PublicationError::new(error.to_string()))?
        .to_owned();
    let release = Release {
        commit: commit.to_string(),
        version: declaration.version.clone(),
        tag,
        notes,
    };
    let tag_publisher = GitTagPublisher::new(repository_path, remote);

    // Every artifact must be obtainable without building before anything is written.
    let assets = declaration
        .publishers
        .github_release
        .iter()
        .flat_map(|(name, publisher)| {
            publisher.assets.iter().map(move |(asset, path)| {
                (
                    format!("github-release.{name} asset {asset}"),
                    path.as_str(),
                )
            })
        });
    let images = declaration
        .publishers
        .oci_image
        .iter()
        .flat_map(|(name, publisher)| {
            publisher.platforms.iter().map(move |(platform, path)| {
                (format!("oci-image.{name} image {platform}"), path.as_str())
            })
        });
    let charts = declaration
        .publishers
        .oci_helm
        .iter()
        .map(|(name, publisher)| (format!("oci-helm.{name} chart"), publisher.chart.as_str()));
    crate::artifacts::fetch(images.chain(charts).chain(assets))
        .map_err(|error| PublicationError::new(error.to_string()))?;

    let (tag_publisher, trusted_predecessor) = match tag_publisher.state(&release)? {
        RemoteState::Missing => {
            // The tag is created last, so its tagger identity is checked with everything else.
            let tag_publisher = tag_publisher.with_tagger()?;
            let (parent_version, trusted_predecessor) =
                verify_candidate_predecessor(repository_path, commit, &declaration, remote, true)?;
            validate_candidate(
                repository_path,
                commit,
                &declaration,
                &parent_version,
                &changelog,
                analyzer,
            )?;
            (tag_publisher, trusted_predecessor)
        }
        RemoteState::Matching => {
            let (_, trusted_predecessor) =
                verify_candidate_predecessor(repository_path, commit, &declaration, remote, false)?;
            (tag_publisher, trusted_predecessor)
        }
        RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
    };

    let mut publishers = Vec::new();
    for (name, publisher) in &declaration.publishers.oci_image {
        publishers.push(PublisherNode::new(
            format!("oci-image.{name}"),
            Box::new(
                crate::publishers::oci::image::OciImagePublisher::from_declaration(publisher)?
                    .with_trusted_predecessor(trusted_predecessor.clone()),
            ),
            publisher.after.clone(),
        ));
    }
    for (name, publisher) in &declaration.publishers.oci_helm {
        publishers.push(PublisherNode::new(
            format!("oci-helm.{name}"),
            Box::new(
                crate::publishers::oci::helm::OciHelmPublisher::from_declaration(
                    publisher,
                    &release.version,
                )?,
            ),
            publisher.after.clone(),
        ));
    }
    for (name, publisher) in &declaration.publishers.github_release {
        publishers.push(PublisherNode::new(
            format!("github-release.{name}"),
            Box::new(GithubPublisher::from_declaration(
                publisher,
                &declaration.changelog.path,
            )?),
            publisher.after.clone(),
        ));
    }
    progress(PublishEvent::Started {
        tag: &release.tag,
        commit: &release.commit,
    });
    let (tag_step, outcomes) = publish_ordered_with_jobs(
        &tag_publisher,
        &release,
        &publishers,
        jobs,
        fail_fast,
        progress,
    )?;
    Ok(PublishOutcome::Published(PublishReport {
        commit: release.commit,
        tag: release.tag,
        tag_step,
        publisher_outcomes: publishers
            .into_iter()
            .zip(outcomes)
            .map(|(publisher, outcome)| (publisher.name, outcome))
            .collect(),
    }))
}

/// Publishes the tag, then completes every named publisher, and reports what each step found,
/// passing each step to `progress` as soon as it is complete.
pub fn publish_ordered(
    tag_publisher: &dyn TagPublisher,
    release: &Release,
    publishers: &[PublisherNode],
    fail_fast: bool,
    progress: &mut Progress<'_>,
) -> Result<(TagStep, Vec<PublisherOutcome>), PublicationError> {
    publish_ordered_with_jobs(
        tag_publisher,
        release,
        publishers,
        NonZeroUsize::MIN,
        fail_fast,
        progress,
    )
}

pub fn publish_ordered_with_jobs(
    tag_publisher: &dyn TagPublisher,
    release: &Release,
    publishers: &[PublisherNode],
    jobs: NonZeroUsize,
    fail_fast: bool,
    progress: &mut Progress<'_>,
) -> Result<(TagStep, Vec<PublisherOutcome>), PublicationError> {
    let tag_state = tag_publisher.state(release);
    let states = publisher_states(release, publishers, jobs);
    let tag_step = match tag_state? {
        RemoteState::Missing => TagStep::Pushed,
        RemoteState::Matching => TagStep::AlreadyPushed,
        RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
    };
    for state in &states {
        match state {
            Ok(PublisherState::Conflicting(message)) => {
                return Err(PublicationError::new(message));
            }
            Err(error) => return Err(error.clone()),
            Ok(_) => {}
        }
    }
    let states: Vec<_> = states.into_iter().map(Result::unwrap).collect();

    tag_publisher.publish(release)?;
    progress(PublishEvent::Tag {
        tag: &release.tag,
        step: tag_step,
    });

    enum Execution {
        Pending,
        Running,
        Succeeded,
        Failed,
        Skipped { failed: String },
    }

    let indexes: HashMap<&str, usize> = publishers
        .iter()
        .enumerate()
        .map(|(index, publisher)| (publisher.name.as_str(), index))
        .collect();
    let mut executions: Vec<_> = publishers.iter().map(|_| Execution::Pending).collect();
    let mut outcomes: Vec<Option<PublisherOutcome>> = publishers.iter().map(|_| None).collect();
    let mut first_failure = None;
    let progress = Mutex::new(progress);

    thread::scope(|scope| -> Result<(), PublicationError> {
        let (completed, completions) = mpsc::channel();
        let mut running = 0;

        while executions
            .iter()
            .any(|execution| matches!(execution, Execution::Pending | Execution::Running))
        {
            let mut skipped = false;
            for index in 0..publishers.len() {
                if !matches!(executions[index], Execution::Pending) {
                    continue;
                }
                let failed = if fail_fast {
                    first_failure.clone()
                } else {
                    publishers[index].after.iter().find_map(|dependency| {
                        let dependency = indexes.get(dependency.as_str())?;
                        match &executions[*dependency] {
                            Execution::Failed => Some(publishers[*dependency].name.clone()),
                            Execution::Skipped { failed } => Some(failed.clone()),
                            Execution::Pending | Execution::Running | Execution::Succeeded => None,
                        }
                    })
                };
                if let Some(failed) = failed {
                    let reason = if fail_fast {
                        format!("skipped: fail-fast after {failed} failed")
                    } else {
                        format!("skipped: {failed} failed")
                    };
                    report_publish_event(
                        &progress,
                        PublishEvent::PublisherSkipped {
                            name: &publishers[index].name,
                            reason: &reason,
                        },
                    );
                    outcomes[index] = Some(PublisherOutcome::Skipped { reason });
                    executions[index] = Execution::Skipped { failed };
                    skipped = true;
                }
            }

            while running < jobs.get() && (!fail_fast || first_failure.is_none()) {
                let ready = publishers
                    .iter()
                    .enumerate()
                    .find_map(|(index, publisher)| {
                        if !matches!(executions[index], Execution::Pending) {
                            return None;
                        }
                        publisher
                            .after
                            .iter()
                            .all(|dependency| {
                                indexes.get(dependency.as_str()).is_some_and(|dependency| {
                                    matches!(executions[*dependency], Execution::Succeeded)
                                })
                            })
                            .then_some(index)
                    });
                let Some(index) = ready else { break };

                executions[index] = Execution::Running;
                running += 1;
                let completed = completed.clone();
                let progress = &progress;
                let publisher = &publishers[index];
                let state = &states[index];
                scope.spawn(move || {
                    let result = panic::catch_unwind(AssertUnwindSafe(|| {
                        complete_publisher(release, publisher, state, progress)
                    }))
                    .unwrap_or_else(|_| {
                        Err(PublicationError::new(format!(
                            "publisher {} panicked",
                            publisher.name
                        )))
                    });
                    completed
                        .send((index, result))
                        .expect("publication scheduler receives every result");
                });
            }

            if running == 0 {
                if skipped {
                    continue;
                }
                if executions
                    .iter()
                    .any(|execution| matches!(execution, Execution::Pending))
                {
                    return Err(PublicationError::new(
                        "publisher dependency graph did not become ready",
                    ));
                }
                break;
            }

            let (index, result) = completions
                .recv()
                .expect("every running publisher reports its result");
            running -= 1;
            match result {
                Ok((step, artifacts)) => {
                    report_publish_event(
                        &progress,
                        PublishEvent::Publisher {
                            name: &publishers[index].name,
                            step,
                        },
                    );
                    outcomes[index] = Some(PublisherOutcome::Published { step, artifacts });
                    executions[index] = Execution::Succeeded;
                }
                Err(error) => {
                    let reason = error.to_string();
                    report_publish_event(
                        &progress,
                        PublishEvent::PublisherFailed {
                            name: &publishers[index].name,
                            reason: &reason,
                        },
                    );
                    outcomes[index] = Some(PublisherOutcome::Failed { reason });
                    executions[index] = Execution::Failed;
                    first_failure.get_or_insert_with(|| publishers[index].name.clone());
                }
            }
        }
        Ok(())
    })?;

    match tag_publisher.state(release)? {
        RemoteState::Matching => {}
        state => {
            return Err(PublicationError::new(format!(
                "Git tag {:?} was not published: {state:?}",
                release.tag
            )));
        }
    }
    Ok((
        tag_step,
        outcomes
            .into_iter()
            .map(|outcome| outcome.expect("every publisher has an outcome"))
            .collect(),
    ))
}

fn publisher_states(
    release: &Release,
    publishers: &[PublisherNode],
    jobs: NonZeroUsize,
) -> Vec<Result<PublisherState, PublicationError>> {
    let next = AtomicUsize::new(0);
    let (checked, checks) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..jobs.get().min(publishers.len()) {
            let checked = checked.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(publisher) = publishers.get(index) else {
                        break;
                    };
                    checked
                        .send((index, publisher.publisher.state(release)))
                        .expect("publication pre-check receives every result");
                }
            });
        }
    });
    drop(checked);

    let mut states: Vec<_> = publishers.iter().map(|_| None).collect();
    for (index, state) in checks {
        states[index] = Some(state);
    }
    states
        .into_iter()
        .map(|state| state.expect("every publisher was checked"))
        .collect()
}

fn complete_publisher(
    release: &Release,
    publisher: &PublisherNode,
    state: &PublisherState,
    progress: &Mutex<&mut Progress<'_>>,
) -> Result<(PublisherStep, ArtifactSteps), PublicationError> {
    let step = match state {
        PublisherState::Published => PublisherStep::AlreadyPublished,
        PublisherState::Missing => PublisherStep::Published,
        PublisherState::Draft => PublisherStep::DraftCompleted,
        PublisherState::Incomplete | PublisherState::AliasesPending => {
            PublisherStep::ArtifactsCompleted
        }
        PublisherState::Conflicting(_) => unreachable!("conflicts were checked before the tag"),
    };
    let artifacts = if step == PublisherStep::AlreadyPublished {
        let artifacts = publisher.publisher.complete_steps(release)?;
        for artifact in &artifacts {
            report_publish_event(progress, PublishEvent::Artifact(artifact));
        }
        artifacts
    } else {
        publisher
            .publisher
            .publish_with_progress(release, &mut |artifact| {
                report_publish_event(progress, PublishEvent::Artifact(artifact));
            })?
    };
    match publisher.publisher.state(release)? {
        PublisherState::Published | PublisherState::AliasesPending => {}
        state => {
            return Err(PublicationError::new(format!(
                "publisher {} did not complete {:?}: {state:?}",
                publisher.name, release.tag
            )));
        }
    }
    Ok((step, artifacts))
}

fn report_publish_event(progress: &Mutex<&mut Progress<'_>>, event: PublishEvent<'_>) {
    progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())(event);
}

pub fn validate_new_release(
    version: &Version,
    parent_version: &Version,
    final_base: Option<&Version>,
    stable: bool,
    analysis: &Analysis,
    changelog: &str,
    heading: &ChangelogHeading,
) -> Result<(), PublicationError> {
    read_version_section(changelog, heading, version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    if !is_valid_successor(parent_version, version) {
        return Err(PublicationError::new(format!(
            "release version {version} is not a valid successor of {parent_version}"
        )));
    }
    if let Some(minimum) = analysis
        .next_version(final_base, stable)
        .map_err(|error| PublicationError::new(error.to_string()))?
        && version_core(version) < version_core(&minimum)
    {
        return Err(PublicationError::new(format!(
            "release version {version} is lower than analyzed minimum {minimum}"
        )));
    }
    Ok(())
}

pub fn is_valid_successor(parent: &Version, candidate: &Version) -> bool {
    if candidate <= parent {
        return false;
    }

    let parent_core = version_core(parent);
    let candidate_core = version_core(candidate);
    if !parent.pre.is_empty() && candidate_core == parent_core {
        return true;
    }

    parent
        .patch
        .checked_add(1)
        .is_some_and(|patch| candidate_core == [parent.major, parent.minor, patch])
        || parent
            .minor
            .checked_add(1)
            .is_some_and(|minor| candidate_core == [parent.major, minor, 0])
        || parent
            .major
            .checked_add(1)
            .is_some_and(|major| candidate_core == [major, 0, 0])
}

/// Verify the candidate's predecessor; returns the parent's version, and that version again if the
/// release commit trusts it as a predecessor released by another tool.
fn verify_candidate_predecessor(
    repository_path: &Path,
    commit: Oid,
    declaration: &ReleaseDeclaration,
    remote: &str,
    verify_completion: bool,
) -> Result<(Version, Option<Version>), PublicationError> {
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!(
            "could not open Git repository {}: {error}",
            repository_path.display()
        ))
    })?;
    let candidate = repository.find_commit(commit).map_err(|error| {
        PublicationError::new(format!("could not read release commit {commit}: {error}"))
    })?;
    let trusted_predecessor = TrustedPredecessor::from_commit_message(
        candidate
            .message()
            .map_err(|_| PublicationError::new("release commit message is not valid UTF-8"))?,
    )
    .map_err(|error| PublicationError::new(error.to_string()))?;
    let parent = if candidate.parent_count() == 0 {
        None
    } else {
        Some(candidate.parent_id(0).map_err(|error| {
            PublicationError::new(format!("could not read first parent of {commit}: {error}"))
        })?)
    };
    let parent_version = match parent {
        Some(parent) => {
            evaluate_release_declaration(repository_path, &parent.to_string())
                .map_err(|error| PublicationError::new(error.to_string()))?
                .declaration
                .version
        }
        None => Version::new(0, 0, 0),
    };
    verify_predecessor_with_trust_mode(
        repository_path,
        commit,
        &parent_version,
        declaration,
        remote,
        trusted_predecessor.as_ref(),
        verify_completion,
        None,
    )?;
    // The trusted tag was verified to be the parent version's tag.
    let trusted_version = trusted_predecessor.map(|_| parent_version.clone());
    Ok((parent_version, trusted_version))
}

fn validate_candidate(
    repository_path: &Path,
    commit: Oid,
    declaration: &ReleaseDeclaration,
    parent_version: &Version,
    changelog: &str,
    analyzer: &dyn Analyzer,
) -> Result<(), PublicationError> {
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!(
            "could not open Git repository {}: {error}",
            repository_path.display()
        ))
    })?;
    let final_base = final_base(&repository, commit, declaration)?;
    let range = final_base
        .as_ref()
        .map_or_else(
            || CommitRange::initial(repository_path, commit.to_string()),
            |(_, base)| CommitRange::between(repository_path, base.to_string(), commit.to_string()),
        )
        .map_err(|error| PublicationError::new(error.to_string()))?
        .excluding(commit);
    let analysis = analyzer
        .analyze(&range, &declaration.analyzer, &declaration.tag)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    validate_new_release(
        &declaration.version,
        parent_version,
        final_base.as_ref().map(|(version, _)| version),
        declaration.stable,
        &analysis,
        changelog,
        &declaration
            .changelog
            .heading()
            .map_err(|error| PublicationError::new(error.to_string()))?,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPredecessor {
    pub version: Version,
    pub tag: String,
    pub commit: Oid,
    /// The release commit is not published yet: its tag is missing locally and on the remote.
    pub pending: bool,
}

pub fn verify_predecessor(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    remote: &str,
) -> Result<Option<VerifiedPredecessor>, PublicationError> {
    verify_predecessor_with_trust(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        remote,
        None,
    )
}

pub fn verify_predecessor_with_trust(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    remote: &str,
    trusted_predecessor: Option<&TrustedPredecessor>,
) -> Result<Option<VerifiedPredecessor>, PublicationError> {
    verify_predecessor_with_trust_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        remote,
        trusted_predecessor,
        true,
        None,
    )
}

/// Verify the predecessor of `HEAD` for `plan` and `prepare`. Unlike [`verify_predecessor_with_trust`],
/// it accepts a release commit of `previous_version` whose tag is missing locally:
///
/// - If the remote has no such tag, it is a pending release. Its own declaration stands in for the
///   missing tag, since `publish` will tag exactly that commit.
/// - If the remote tag points to it, the release is published but its tag not fetched yet; it is
///   verified like any published predecessor, using the remote tag.
pub fn verify_head_predecessor(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    remote: &str,
    trusted_predecessor: Option<&TrustedPredecessor>,
) -> Result<Option<VerifiedPredecessor>, PublicationError> {
    let tag_publisher = GitTagPublisher::new(repository_path, remote);
    let untagged = pending_predecessor(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        &tag_publisher,
        |commit| {
            match evaluate_release_declaration(repository_path, &commit.to_string()) {
                Ok(evaluated) => Ok(Some(evaluated.declaration)),
                // A commit without a declaration cannot be the parent of a Presswerk release commit.
                Err(
                    EvaluationError::MissingDeclaration(_) | EvaluationError::MissingLockFile(_),
                ) => Ok(None),
                Err(error) => Err(PublicationError::new(error.to_string())),
            }
        },
    )?;
    let published_commit = match untagged {
        Some(UntaggedRelease::Pending(pending)) => {
            if trusted_predecessor.is_some() {
                return Err(PublicationError::new(format!(
                    "--trust-predecessor is unnecessary: pending release {} at {} is not published yet",
                    pending.tag,
                    short_oid(pending.commit)
                )));
            }
            return Ok(Some(pending));
        }
        Some(UntaggedRelease::Published(commit)) => Some(commit),
        None => None,
    };
    verify_predecessor_with_trust_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        remote,
        trusted_predecessor,
        true,
        published_commit,
    )
}

/// A release commit of the previous version whose tag is missing locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntaggedRelease {
    /// The remote has no tag either.
    Pending(VerifiedPredecessor),
    /// The remote tag points to this release commit; its completion is not verified yet.
    Published(Oid),
}

/// Find the release commit that `candidate` builds on, if `previous_version` has no local tag.
///
/// The pending release commit is the first-parent ancestor of `candidate` where `previous_version`
/// starts, so its parent carries a lower version. `declaration_at` reads the declaration of a
/// commit, or `None` if the commit has none.
pub fn pending_predecessor<D>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    mut declaration_at: D,
) -> Result<Option<UntaggedRelease>, PublicationError>
where
    D: FnMut(Oid) -> Result<Option<ReleaseDeclaration>, PublicationError>,
{
    // The implicit 0.0.0 baseline is never released, so it cannot be pending. A candidate with
    // another version than its predecessor builds on a release commit below it, never a pending one.
    if *previous_version == Version::new(0, 0, 0)
        || current_declaration.version != *previous_version
    {
        return Ok(None);
    }
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!("could not open Git repository: {error}"))
    })?;
    let previous_tag = current_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    if local_tagged_commit(&repository, &previous_tag)?.is_some() {
        return Ok(None);
    }

    let first_parent = |commit: Oid| -> Result<Option<Oid>, PublicationError> {
        let commit = repository.find_commit(commit).map_err(|error| {
            PublicationError::new(format!("could not read Git commit {commit}: {error}"))
        })?;
        if commit.parent_count() == 0 {
            return Ok(None);
        }
        commit.parent_id(0).map(Some).map_err(|error| {
            PublicationError::new(format!(
                "could not read first parent of Git commit {}: {error}",
                commit.id()
            ))
        })
    };
    let mut pending = candidate;
    let mut pending_declaration = current_declaration.clone();
    loop {
        // A root commit is where the project started, not a release commit.
        let Some(parent) = first_parent(pending)? else {
            return Ok(None);
        };
        let Some(parent_declaration) = declaration_at(parent)? else {
            return Ok(None);
        };
        if parent_declaration.version == *previous_version {
            pending = parent;
            pending_declaration = parent_declaration;
            continue;
        }
        if parent_declaration.version > *previous_version {
            return Ok(None);
        }
        break;
    }
    let declared_tag = pending_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    if declared_tag != previous_tag {
        return Err(PublicationError::new(format!(
            "pending release at {pending} declares tag {declared_tag:?}, expected {previous_tag:?}"
        )));
    }
    let release = Release {
        commit: pending.to_string(),
        version: previous_version.clone(),
        tag: previous_tag.clone(),
        notes: String::new(),
    };
    match tag_publisher.state(&release)? {
        RemoteState::Missing => {}
        RemoteState::Matching => return Ok(Some(UntaggedRelease::Published(pending))),
        RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
    }
    Ok(Some(UntaggedRelease::Pending(VerifiedPredecessor {
        version: previous_version.clone(),
        tag: previous_tag,
        commit: pending,
        pending: true,
    })))
}

pub(crate) fn short_oid(commit: Oid) -> String {
    let mut commit = commit.to_string();
    commit.truncate(7);
    commit
}

#[allow(clippy::too_many_arguments)]
fn verify_predecessor_with_trust_mode(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    remote: &str,
    trusted_predecessor: Option<&TrustedPredecessor>,
    verify_completion: bool,
    published_commit: Option<Oid>,
) -> Result<Option<VerifiedPredecessor>, PublicationError> {
    let tag_publisher = GitTagPublisher::new(repository_path, remote);
    let previous_tag = current_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    verify_predecessor_with_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        &tag_publisher,
        trusted_predecessor,
        verify_completion,
        published_commit,
        |commit| match evaluate_release_declaration(repository_path, &commit.to_string()) {
            Ok(evaluated) => Ok(evaluated.declaration),
            Err(EvaluationError::Declaration(DeclarationError::UnsupportedSchemaVersion(
                schema_version,
            ))) => Err(PublicationError::unsupported_predecessor(
                previous_tag.clone(),
                commit,
                schema_version,
            )),
            Err(EvaluationError::Declaration(DeclarationError::IncompatibleUnstableSchema(_))) => {
                Err(PublicationError::unsupported_predecessor(
                    previous_tag.clone(),
                    commit,
                    UNSTABLE_SCHEMA_VERSION,
                ))
            }
            // Presswerk never released a commit without a lock file, so it has no declaration either.
            Err(EvaluationError::MissingDeclaration(_) | EvaluationError::MissingLockFile(_)) => {
                Err(PublicationError::undeclared_predecessor(
                    previous_tag.clone(),
                    commit,
                ))
            }
            Err(error) => Err(PublicationError::new(error.to_string())),
        },
        |publisher, tag, _version| {
            GithubClient::from_environment(&publisher.api_url, &publisher.token.secret)?
                .release_state(&publisher.repository, tag)
        },
        |publisher, tag| {
            registry_manifest(
                &publisher.repository,
                publisher.public,
                &publisher.username,
                &publisher.password,
                tag,
            )
        },
        |publisher, tag| {
            registry_manifest(
                &publisher.repository,
                publisher.public,
                &publisher.username,
                &publisher.password,
                tag,
            )
        },
    )
}

fn registry_manifest(
    repository: &str,
    public: bool,
    username: &crate::declaration::Username,
    password: &crate::declaration::Token,
    tag: &str,
) -> Result<Option<Vec<u8>>, PublicationError> {
    // A public repository is read anonymously, so its credentials are never needed.
    let credentials = if public {
        None
    } else {
        Some(crate::publishers::oci::Credentials::from_environment(
            username,
            &password.secret,
        )?)
    };
    Ok(
        crate::publishers::oci::RegistryClient::new(repository, credentials)?
            .manifest(tag)?
            .map(|(_, manifest)| manifest),
    )
}

#[allow(clippy::too_many_arguments)]
///
/// `release_state` reads a GitHub Release by tag; `oci_image_manifest` reads the manifest an OCI tag
/// points to, or `None` if the registry has no such tag.
pub fn verify_predecessor_with<E, F, O>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    trusted_predecessor: Option<&TrustedPredecessor>,
    evaluate_declaration: E,
    release_state: F,
    oci_image_manifest: O,
) -> Result<Option<VerifiedPredecessor>, PublicationError>
where
    E: FnMut(Oid) -> Result<ReleaseDeclaration, PublicationError>,
    F: FnMut(
        &GithubReleaseDeclaration,
        &str,
        &Version,
    ) -> Result<GithubReleaseState, PublicationError>,
    O: FnMut(&OciImageDeclaration, &str) -> Result<Option<Vec<u8>>, PublicationError>,
{
    verify_predecessor_with_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        tag_publisher,
        trusted_predecessor,
        true,
        None,
        evaluate_declaration,
        release_state,
        oci_image_manifest,
        |publisher, _| {
            Err(PublicationError::new(format!(
                "no OCI Helm manifest reader was provided for {}",
                publisher.repository
            )))
        },
    )
}

/// Verify a predecessor while supplying readers for every built-in publisher.
#[allow(clippy::too_many_arguments)]
pub fn verify_predecessor_with_oci_helm<E, F, O, H>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    trusted_predecessor: Option<&TrustedPredecessor>,
    evaluate_declaration: E,
    release_state: F,
    oci_image_manifest: O,
    oci_helm_manifest: H,
) -> Result<Option<VerifiedPredecessor>, PublicationError>
where
    E: FnMut(Oid) -> Result<ReleaseDeclaration, PublicationError>,
    F: FnMut(
        &GithubReleaseDeclaration,
        &str,
        &Version,
    ) -> Result<GithubReleaseState, PublicationError>,
    O: FnMut(&OciImageDeclaration, &str) -> Result<Option<Vec<u8>>, PublicationError>,
    H: FnMut(&OciHelmDeclaration, &str) -> Result<Option<Vec<u8>>, PublicationError>,
{
    verify_predecessor_with_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        tag_publisher,
        trusted_predecessor,
        true,
        None,
        evaluate_declaration,
        release_state,
        oci_image_manifest,
        oci_helm_manifest,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_predecessor_with_mode<E, F, O, H>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    trusted_predecessor: Option<&TrustedPredecessor>,
    verify_completion: bool,
    // The commit the remote tag points to, when the tag is published but not fetched.
    published_commit: Option<Oid>,
    mut evaluate_declaration: E,
    mut release_state: F,
    mut oci_image_manifest: O,
    mut oci_helm_manifest: H,
) -> Result<Option<VerifiedPredecessor>, PublicationError>
where
    E: FnMut(Oid) -> Result<ReleaseDeclaration, PublicationError>,
    F: FnMut(
        &GithubReleaseDeclaration,
        &str,
        &Version,
    ) -> Result<GithubReleaseState, PublicationError>,
    O: FnMut(&OciImageDeclaration, &str) -> Result<Option<Vec<u8>>, PublicationError>,
    H: FnMut(&OciHelmDeclaration, &str) -> Result<Option<Vec<u8>>, PublicationError>,
{
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!("could not open Git repository: {error}"))
    })?;
    let previous_tag = current_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    let local_commit = local_tagged_commit(&repository, &previous_tag)?;
    let Some(previous_commit) = local_commit.or(published_commit) else {
        if *previous_version == Version::new(0, 0, 0)
            && !has_release_tags(&repository, &current_declaration.tag)?
        {
            if trusted_predecessor.is_some() {
                return Err(PublicationError::new(
                    "--trust-predecessor is unnecessary for the first release",
                ));
            }
            return Ok(None);
        }
        return Err(PublicationError::new(format!(
            "previous release tag {previous_tag:?} is missing"
        )));
    };
    if previous_commit != candidate
        && !repository
            .graph_descendant_of(candidate, previous_commit)
            .map_err(|error| {
                PublicationError::new(format!(
                    "could not check previous release ancestry: {error}"
                ))
            })?
    {
        return Err(PublicationError::new(format!(
            "previous release tag {previous_tag:?} at {previous_commit} is not an ancestor of {candidate}"
        )));
    }

    let previous_declaration = match evaluate_declaration(previous_commit) {
        Ok(declaration) => {
            if trusted_predecessor.is_some() {
                return Err(PublicationError::new(format!(
                    "trusted predecessor {previous_tag}={previous_commit} is unnecessary because its declaration is readable"
                )));
            }
            declaration
        }
        Err(error) => {
            let Some(unreadable) = error.as_unreadable_predecessor() else {
                return Err(error);
            };
            if unreadable.tag != previous_tag || unreadable.commit != previous_commit {
                return Err(error);
            }
            let Some(trusted) = trusted_predecessor else {
                return Err(error);
            };
            if trusted.tag != previous_tag || trusted.commit != previous_commit {
                return Err(PublicationError::new(format!(
                    "trusted predecessor {}={} does not match expected predecessor {previous_tag}={previous_commit}",
                    trusted.tag, trusted.commit
                )));
            }
            // A trusted tag may be lightweight, e.g. when another tool created it.
            return verified_remote_predecessor(
                tag_publisher,
                previous_version,
                previous_tag,
                previous_commit,
                true,
            )
            .map(Some);
        }
    };
    if previous_declaration.version != *previous_version {
        return Err(PublicationError::new(format!(
            "previous release tag {previous_tag:?} contains version {}, expected {previous_version}",
            previous_declaration.version
        )));
    }
    let declared_tag = previous_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    if declared_tag != previous_tag {
        return Err(PublicationError::new(format!(
            "previous release at {previous_commit} declares tag {declared_tag:?}, expected {previous_tag:?}"
        )));
    }

    if !verify_completion {
        return Ok(Some(VerifiedPredecessor {
            version: previous_version.clone(),
            tag: previous_tag,
            commit: previous_commit,
            pending: false,
        }));
    }

    let verified = verified_remote_predecessor(
        tag_publisher,
        previous_version,
        previous_tag.clone(),
        previous_commit,
        false,
    )?;

    let expected_prerelease = !previous_version.pre.is_empty();
    for publisher in previous_declaration.publishers.github_release.values() {
        match release_state(publisher, &previous_tag, previous_version)? {
            GithubReleaseState::Published { prerelease } if prerelease == expected_prerelease => {}
            state => {
                return Err(PublicationError::new(format!(
                    "previous GitHub Release {previous_tag:?} in {} is incomplete: {state:?}",
                    publisher.repository
                )));
            }
        }
    }
    // Only version tags count: aliases are best effort and may point to another release.
    for (name, publisher) in &previous_declaration.publishers.oci_image {
        let tags = crate::publishers::oci::image::render_tags(
            &publisher.tags,
            &publisher.aliases,
            previous_version,
        )
        .map_err(|error| PublicationError::new(format!("oci-image.{name}: {error}")))?;
        for tag in tags {
            let crate::publishers::oci::image::OciTag::Version(tag) = tag else {
                continue;
            };
            let found = match oci_image_manifest(publisher, &tag)? {
                None => "the tag is missing".to_owned(),
                Some(manifest) => match crate::publishers::oci::image::annotated_version(&manifest)
                {
                    Some(version) if version == *previous_version => continue,
                    Some(version) => format!("the tag points to version {version}"),
                    None => "the tag does not point to an image index with a version annotation"
                        .to_owned(),
                },
            };
            return Err(PublicationError::new(format!(
                "previous OCI image release {}:{tag} (oci-image.{name}) is incomplete: {found}, expected version {previous_version}",
                publisher.repository
            )));
        }
    }
    for (name, publisher) in &previous_declaration.publishers.oci_helm {
        let tag = crate::publishers::oci::helm::tag(previous_version);
        let found = match oci_helm_manifest(publisher, &tag)? {
            None => "the tag is missing".to_owned(),
            Some(manifest)
                if crate::publishers::oci::helm::is_release_manifest(
                    &manifest,
                    previous_version,
                    &publisher.chart,
                ) =>
            {
                continue;
            }
            Some(_) => "the tag does not point to this release's Helm chart".to_owned(),
        };
        return Err(PublicationError::new(format!(
            "previous OCI Helm release {}:{tag} (oci-helm.{name}) is incomplete: {found}, expected version {previous_version} from {}",
            publisher.repository, publisher.chart
        )));
    }
    Ok(Some(verified))
}

fn verified_remote_predecessor(
    tag_publisher: &dyn TagPublisher,
    previous_version: &Version,
    previous_tag: String,
    previous_commit: Oid,
    accept_lightweight: bool,
) -> Result<VerifiedPredecessor, PublicationError> {
    let previous = Release {
        commit: previous_commit.to_string(),
        version: previous_version.clone(),
        tag: previous_tag.clone(),
        notes: String::new(),
    };
    match tag_publisher.state(&previous)? {
        RemoteState::Matching => {}
        RemoteState::Missing => {
            return Err(PublicationError::new(format!(
                "previous release tag {previous_tag:?} is missing from the remote"
            )));
        }
        RemoteState::Conflicting(message) => {
            if !accept_lightweight
                || tag_publisher.lightweight_target(&previous_tag)? != Some(previous_commit)
            {
                return Err(PublicationError::new(message));
            }
        }
    }
    Ok(VerifiedPredecessor {
        version: previous_version.clone(),
        tag: previous_tag,
        commit: previous_commit,
        pending: false,
    })
}

fn final_base(
    repository: &Repository,
    candidate: Oid,
    declaration: &ReleaseDeclaration,
) -> Result<Option<(Version, Oid)>, PublicationError> {
    let names = repository
        .tag_names(None)
        .map_err(|error| PublicationError::new(format!("could not list Git tags: {error}")))?;
    let mut final_release = None;
    for name in names.iter_bytes() {
        let Ok(name) = std::str::from_utf8(name) else {
            continue;
        };
        let Some(version) = declaration
            .tag
            .parse(name)
            .map_err(|error| PublicationError::new(error.to_string()))?
        else {
            continue;
        };
        let Some(commit) = local_tagged_commit(repository, name)? else {
            continue;
        };
        if commit != candidate
            && !repository
                .graph_descendant_of(candidate, commit)
                .map_err(|error| {
                    PublicationError::new(format!("could not check Git tag ancestry: {error}"))
                })?
        {
            continue;
        }
        if version > declaration.version {
            return Err(PublicationError::new(format!(
                "reachable release tag {name:?} has version {version}, higher than candidate {}",
                declaration.version
            )));
        }
        if !version.pre.is_empty() || version == declaration.version {
            continue;
        }
        if final_release
            .as_ref()
            .is_none_or(|(current, _): &(Version, Oid)| version > *current)
        {
            final_release = Some((version, commit));
        }
    }
    Ok(final_release)
}

fn local_tagged_commit(
    repository: &Repository,
    tag: &str,
) -> Result<Option<Oid>, PublicationError> {
    let reference = format!("refs/tags/{tag}");
    match repository.find_reference(&reference) {
        Ok(reference) => reference
            .peel_to_commit()
            .map(|commit| Some(commit.id()))
            .map_err(|error| {
                PublicationError::new(format!(
                    "Git tag {tag:?} does not resolve to a commit: {error}"
                ))
            }),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(PublicationError::new(format!(
            "could not read Git tag {tag:?}: {error}"
        ))),
    }
}

fn has_release_tags(
    repository: &Repository,
    tag: &crate::declaration::Tag,
) -> Result<bool, PublicationError> {
    let names = repository
        .tag_names(None)
        .map_err(|error| PublicationError::new(format!("could not list Git tags: {error}")))?;
    for name in names.iter_bytes() {
        let Ok(name) = std::str::from_utf8(name) else {
            continue;
        };
        if tag
            .parse(name)
            .map_err(|error| PublicationError::new(error.to_string()))?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_release_publisher_tags(
    declaration: &ReleaseDeclaration,
) -> Result<(), PublicationError> {
    declaration
        .validate_publisher_tags(&declaration.version)
        .map_err(|error| PublicationError::new(error.to_string()))
}

fn read_text_at(
    repository_path: &Path,
    commit: Oid,
    path: &str,
) -> Result<String, PublicationError> {
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!("could not open Git repository: {error}"))
    })?;
    let commit = repository
        .find_commit(commit)
        .map_err(|error| PublicationError::new(format!("could not read Git commit: {error}")))?;
    let tree = commit
        .tree()
        .map_err(|error| PublicationError::new(format!("could not read Git tree: {error}")))?;
    let entry = tree.get_path(Path::new(path)).map_err(|error| {
        PublicationError::new(format!(
            "could not read {path} from commit {}: {error}",
            commit.id()
        ))
    })?;
    let blob = entry
        .to_object(&repository)
        .and_then(|object| object.peel_to_blob())
        .map_err(|error| {
            PublicationError::new(format!(
                "{path} in commit {} is not a file: {error}",
                commit.id()
            ))
        })?;
    std::str::from_utf8(blob.content())
        .map(str::to_owned)
        .map_err(|error| {
            PublicationError::new(format!(
                "{path} in commit {} is not UTF-8: {error}",
                commit.id()
            ))
        })
}

fn version_core(version: &Version) -> [u64; 3] {
    [version.major, version.minor, version.patch]
}

#[cfg(test)]
mod tests {
    use super::validate_release_publisher_tags;
    use crate::declaration::ReleaseDeclaration;

    #[test]
    fn publication_rejects_actual_version_registry_tag_collisions() {
        let declaration = ReleaseDeclaration::from_json(
            r#"{
              "schemaVersion": 0,
              "version": "0.1.0",
              "stable": false,
              "versionFiles": [],
              "tag": { "format": "v{version}" },
              "analyzer": {
                "convention": "gitmoji",
                "gitmoji": { "catalog": {} }
              },
              "changelog": { "path": "CHANGELOG.md" },
              "publishers": {
                "github-release": {},
                "oci-image": {
                  "image": {
                    "repository": "ghcr.io/owner/project",
                    "platforms": {
                      "linux/amd64": "/nix/store/00000000000000000000000000000000-image.tar.gz"
                    },
                    "tags": ["v{version}"],
                    "aliases": ["0.1.0"],
                    "username": { "secret": "REGISTRY_USERNAME" },
                    "password": { "secret": "REGISTRY_TOKEN" }
                  }
                },
                "oci-helm": {
                  "chart": {
                    "repository": "ghcr.io/owner/project",
                    "chart": "/nix/store/00000000000000000000000000000000-chart.tgz",
                    "username": { "secret": "REGISTRY_USERNAME" },
                    "password": { "secret": "REGISTRY_TOKEN" }
                  }
                }
              }
            }"#,
        )
        .unwrap();

        let error = validate_release_publisher_tags(&declaration)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("oci-image.image and oci-helm.chart"),
            "{error}"
        );
        assert!(error.contains("both render 0.1.0"), "{error}");
    }
}
