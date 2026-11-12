use crate::analysis::{Analysis, Analyzer, CommitRange, GitCliffAnalyzer};
use crate::changelog::read_version_section;
use crate::classification::{CommitClassification, classify_commit};
use crate::declaration::{
    DeclarationError, GithubReleasePublisher as GithubReleaseDeclaration, ReleaseDeclaration,
    UNSTABLE_SCHEMA_VERSION,
};
use crate::evaluation::{EvaluationError, evaluate_release_declaration};
use crate::release_commit::TrustedPredecessor;
use git2::{ErrorCode, ObjectType, Oid, Repository};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedPredecessor {
    pub tag: String,
    pub commit: Oid,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationError {
    message: String,
    unsupported_predecessor: Option<UnsupportedPredecessor>,
}

impl PublicationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unsupported_predecessor: None,
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
        Self {
            message,
            unsupported_predecessor: Some(UnsupportedPredecessor {
                tag,
                commit,
                schema_version,
            }),
        }
    }

    #[must_use]
    pub fn as_unsupported_predecessor(&self) -> Option<&UnsupportedPredecessor> {
        self.unsupported_predecessor.as_ref()
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
    Conflicting(String),
}

/// What happened to one artifact of a publisher, such as a GitHub Release asset or an OCI tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetStep {
    Uploaded,
    AlreadyUploaded,
    /// Deliberately left as it is, such as an alias that points to a higher version.
    Kept(String),
}

/// The kind of artifact a publisher reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A GitHub Release asset.
    Asset,
    /// An OCI image for one platform.
    Image,
    /// An OCI tag that names the version.
    Tag,
    /// An OCI tag that follows the highest stable version, such as `latest`.
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

pub trait TagPublisher {
    fn state(&self, release: &Release) -> Result<RemoteState, PublicationError>;
    fn publish(&self, release: &Release) -> Result<(), PublicationError>;
}

pub trait Publisher {
    fn state(&self, release: &Release) -> Result<PublisherState, PublicationError>;
    /// Create every missing part and report each artifact.
    fn publish(&self, release: &Release) -> Result<ArtifactSteps, PublicationError>;
    /// Report the artifacts of a publisher whose state is already published, without writing.
    fn complete_steps(&self, _release: &Release) -> Result<ArtifactSteps, PublicationError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubReleaseState {
    Missing,
    Draft { prerelease: bool },
    Published { prerelease: bool },
}

pub trait GithubReleaseReader {
    fn release_state(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<GithubReleaseState, PublicationError>;
}

#[derive(Clone)]
pub struct GithubClient {
    agent: ureq::Agent,
    api_url: Url,
    token: String,
}

impl fmt::Debug for GithubClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GithubClient")
            .field("api_url", &self.api_url)
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl GithubClient {
    pub fn from_environment(api_url: &str, secret: &str) -> Result<Self, PublicationError> {
        let token = env::var(secret).map_err(|_| {
            PublicationError::new(format!(
                "GitHub credential environment variable {secret:?} is not set"
            ))
        })?;
        if token.is_empty() {
            return Err(PublicationError::new(format!(
                "GitHub credential environment variable {secret:?} is empty"
            )));
        }
        Self::new(api_url, token)
    }

    pub fn new(api_url: &str, token: impl Into<String>) -> Result<Self, PublicationError> {
        let api_url = Url::parse(api_url).map_err(|error| {
            PublicationError::new(format!("invalid GitHub API URL {api_url:?}: {error}"))
        })?;
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            api_url,
            token: token.into(),
        })
    }

    fn endpoint(&self, repository: &str, suffix: &[&str]) -> Result<Url, PublicationError> {
        let (owner, name) = repository.split_once('/').ok_or_else(|| {
            PublicationError::new(format!(
                "GitHub repository {repository:?} must use owner/name form"
            ))
        })?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(PublicationError::new(format!(
                "GitHub repository {repository:?} must use owner/name form"
            )));
        }

        let mut url = self.api_url.clone();
        let mut segments = url.path_segments_mut().map_err(|()| {
            PublicationError::new(format!(
                "GitHub API URL {:?} cannot contain path segments",
                self.api_url
            ))
        })?;
        segments.pop_if_empty();
        segments.push("repos").push(owner).push(name);
        for segment in suffix {
            segments.push(segment);
        }
        drop(segments);
        Ok(url)
    }

    fn request(&self, method: &str, url: &Url) -> ureq::Request {
        self.agent
            .request(method, url.as_str())
            .set("Accept", "application/vnd.github+json")
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "presswerk")
    }

    fn validate_repository(&self, repository: &str) -> Result<(), PublicationError> {
        let url = self.endpoint(repository, &[])?;
        match self.request("GET", &url).call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(status, _)) => Err(PublicationError::new(format!(
                "GitHub repository {repository:?} is inaccessible (HTTP {status})"
            ))),
            Err(ureq::Error::Transport(error)) => Err(PublicationError::new(format!(
                "could not inspect GitHub repository {repository:?}: {error}"
            ))),
        }
    }

    fn release(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<Option<GithubRelease>, PublicationError> {
        self.validate_repository(repository)?;
        let url = self.endpoint(repository, &["releases", "tags", tag])?;
        match self.request("GET", &url).call() {
            Ok(response) => response
                .into_json::<GithubRelease>()
                .map(Some)
                .map_err(|error| {
                    PublicationError::new(format!(
                        "could not decode GitHub Release {tag:?} in {repository}: {error}"
                    ))
                }),
            // GitHub finds only published releases by tag; a draft must be looked up in the listing.
            Err(ureq::Error::Status(404, _)) => self.draft_release(repository, tag),
            Err(ureq::Error::Status(status, _)) => Err(PublicationError::new(format!(
                "could not inspect GitHub Release {tag:?} in {repository} (HTTP {status})"
            ))),
            Err(ureq::Error::Transport(error)) => Err(PublicationError::new(format!(
                "could not inspect GitHub Release {tag:?} in {repository}: {error}"
            ))),
        }
    }

    fn draft_release(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<Option<GithubRelease>, PublicationError> {
        const PER_PAGE: usize = 100;
        let mut matches = Vec::new();
        for page in 1.. {
            let mut url = self.endpoint(repository, &["releases"])?;
            url.query_pairs_mut()
                .append_pair("per_page", &PER_PAGE.to_string())
                .append_pair("page", &page.to_string());
            let releases: Vec<ListedGithubRelease> = match self.request("GET", &url).call() {
                Ok(response) => response.into_json().map_err(|error| {
                    PublicationError::new(format!(
                        "could not decode GitHub Releases of {repository}: {error}"
                    ))
                })?,
                Err(ureq::Error::Status(status, _)) => {
                    return Err(PublicationError::new(format!(
                        "could not list GitHub Releases of {repository} (HTTP {status})"
                    )));
                }
                Err(ureq::Error::Transport(error)) => {
                    return Err(PublicationError::new(format!(
                        "could not list GitHub Releases of {repository}: {error}"
                    )));
                }
            };
            let complete = releases.len() < PER_PAGE;
            for listed in releases {
                // An entry without a usable tag could be the release we look for: fail closed.
                if listed.tag_name.is_empty() {
                    return Err(PublicationError::new(format!(
                        "GitHub listed a release without a tag name in {repository}"
                    )));
                }
                if listed.tag_name == tag {
                    matches.push(listed.release);
                }
            }
            if complete {
                break;
            }
        }
        if matches.len() > 1 {
            return Err(PublicationError::new(format!(
                "found {} GitHub Releases for {tag:?} in {repository}, expected at most one",
                matches.len()
            )));
        }
        Ok(matches.pop())
    }

    fn create_draft(
        &self,
        repository: &str,
        release: &Release,
    ) -> Result<GithubRelease, PublicationError> {
        let url = self.endpoint(repository, &["releases"])?;
        let request = GithubReleaseRequest {
            tag_name: &release.tag,
            target_commitish: &release.commit,
            name: &release.tag,
            body: &release.notes,
            draft: true,
            prerelease: !release.version.pre.is_empty(),
        };
        match self.request("POST", &url).send_json(&request) {
            Ok(response) => {
                let created: GithubRelease = response.into_json().map_err(|error| {
                    PublicationError::new(format!(
                        "could not decode created GitHub Release {:?} in {repository}: {error}",
                        release.tag
                    ))
                })?;
                if !created.draft {
                    return Err(PublicationError::new(format!(
                        "GitHub created Release {:?} in {repository} without a draft",
                        release.tag
                    )));
                }
                Ok(created)
            }
            Err(ureq::Error::Status(status, _)) => {
                if let Some(existing) = self.release(repository, &release.tag)? {
                    return Ok(existing);
                }
                Err(PublicationError::new(format!(
                    "could not create draft GitHub Release {:?} in {repository} (HTTP {status})",
                    release.tag
                )))
            }
            Err(ureq::Error::Transport(error)) => {
                if let Some(existing) = self.release(repository, &release.tag)? {
                    return Ok(existing);
                }
                Err(PublicationError::new(format!(
                    "could not create draft GitHub Release {:?} in {repository}: {error}",
                    release.tag
                )))
            }
        }
    }

    fn publish_draft(
        &self,
        repository: &str,
        release: &Release,
        id: u64,
    ) -> Result<GithubRelease, PublicationError> {
        let id = id.to_string();
        let url = self.endpoint(repository, &["releases", &id])?;
        match self
            .request("PATCH", &url)
            .send_json(serde_json::json!({ "draft": false }))
        {
            Ok(response) => response.into_json().map_err(|error| {
                PublicationError::new(format!(
                    "could not decode published GitHub Release {:?} in {repository}: {error}",
                    release.tag
                ))
            }),
            Err(ureq::Error::Status(status, _)) => {
                if let Some(existing) = self.release(repository, &release.tag)?
                    && !existing.draft
                {
                    return Ok(existing);
                }
                Err(PublicationError::new(format!(
                    "could not publish draft GitHub Release {:?} in {repository} (HTTP {status})",
                    release.tag
                )))
            }
            Err(ureq::Error::Transport(error)) => {
                if let Some(existing) = self.release(repository, &release.tag)?
                    && !existing.draft
                {
                    return Ok(existing);
                }
                Err(PublicationError::new(format!(
                    "could not publish draft GitHub Release {:?} in {repository}: {error}",
                    release.tag
                )))
            }
        }
    }
}

impl GithubClient {
    fn upload_asset(
        &self,
        repository: &str,
        release: &Release,
        github_release: &GithubRelease,
        asset: &LocalAsset,
    ) -> Result<(), PublicationError> {
        let template = github_release.upload_url.as_deref().ok_or_else(|| {
            PublicationError::new(format!(
                "GitHub Release {:?} in {repository} has no upload URL",
                release.tag
            ))
        })?;
        // The upload URL is a URI template such as `…/assets{?name,label}`.
        let base = template.split('{').next().unwrap_or(template);
        let mut url = Url::parse(base).map_err(|error| {
            PublicationError::new(format!("invalid GitHub upload URL {template:?}: {error}"))
        })?;
        url.query_pairs_mut().append_pair("name", &asset.name);
        let bytes = std::fs::read(&asset.path).map_err(|error| {
            PublicationError::new(format!(
                "could not read GitHub Release asset {} ({}): {error}",
                asset.name,
                asset.path.display()
            ))
        })?;
        let failure = match self
            .request("POST", &url)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&bytes)
        {
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(status, _)) => format!("HTTP {status}"),
            Err(ureq::Error::Transport(error)) => error.to_string(),
        };
        // A concurrent run may have uploaded the same asset; accept it only if it matches.
        if let Some(current) = self.release(repository, &release.tag)?
            && current.assets.iter().any(|remote| {
                remote.name == asset.name && remote.digest.as_deref() == Some(&asset.digest)
            })
        {
            return Ok(());
        }
        Err(PublicationError::new(format!(
            "could not upload asset {} to GitHub Release {:?} in {repository}: {failure}",
            asset.name, release.tag
        )))
    }
}

impl GithubReleaseReader for GithubClient {
    fn release_state(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<GithubReleaseState, PublicationError> {
        Ok(match self.release(repository, tag)? {
            None => GithubReleaseState::Missing,
            Some(release) if release.draft => GithubReleaseState::Draft {
                prerelease: release.prerelease,
            },
            Some(release) => GithubReleaseState::Published {
                prerelease: release.prerelease,
            },
        })
    }
}

/// A file to attach to a GitHub Release, with the digest GitHub reports for an uploaded asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAsset {
    pub name: String,
    pub path: PathBuf,
    pub digest: String,
}

impl LocalAsset {
    /// Read a fetched artifact; GitHub Release assets must be regular files.
    pub fn from_file(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Result<Self, PublicationError> {
        let name = name.into();
        let path = path.into();
        let bytes = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => std::fs::read(&path),
            Ok(_) => {
                return Err(PublicationError::new(format!(
                    "GitHub Release asset {name} ({}) is not a regular file",
                    path.display()
                )));
            }
            Err(error) => Err(error),
        }
        .map_err(|error| {
            PublicationError::new(format!(
                "could not read GitHub Release asset {name} ({}): {error}",
                path.display()
            ))
        })?;
        let digest =
            Sha256::digest(&bytes)
                .iter()
                .fold(String::from("sha256:"), |mut digest, byte| {
                    use fmt::Write as _;
                    let _ = write!(digest, "{byte:02x}");
                    digest
                });
        Ok(Self { name, path, digest })
    }
}

#[derive(Debug, Clone)]
pub struct GithubPublisher {
    client: GithubClient,
    repository: String,
    assets: Vec<LocalAsset>,
}

impl GithubPublisher {
    /// Build the publisher from its declaration; its assets must already be fetched.
    pub fn from_declaration(
        declaration: &GithubReleaseDeclaration,
    ) -> Result<Self, PublicationError> {
        let assets = declaration
            .assets
            .iter()
            .map(|(name, path)| LocalAsset::from_file(name, path))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            client: GithubClient::from_environment(
                &declaration.api_url,
                &declaration.token.secret,
            )?,
            repository: declaration.repository.clone(),
            assets,
        })
    }

    pub fn new(client: GithubClient, repository: impl Into<String>) -> Self {
        Self {
            client,
            repository: repository.into(),
            assets: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_assets(mut self, assets: Vec<LocalAsset>) -> Self {
        self.assets = assets;
        self
    }

    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        let Some(github_release) = self.client.release(&self.repository, &release.tag)? else {
            return Ok(PublisherState::Missing);
        };
        let expected_prerelease = !release.version.pre.is_empty();
        if github_release.prerelease != expected_prerelease {
            return Ok(PublisherState::Conflicting(format!(
                "GitHub Release {:?} in {} has prerelease={}, expected {expected_prerelease}",
                release.tag, self.repository, github_release.prerelease
            )));
        }
        let mut missing = false;
        for asset in &self.assets {
            match self.remote_asset(release, &github_release, asset) {
                Ok(true) => {}
                Ok(false) => missing = true,
                Err(conflict) => return Ok(PublisherState::Conflicting(conflict)),
            }
        }
        Ok(match (github_release.draft, missing) {
            (true, _) => PublisherState::Draft,
            (false, true) => PublisherState::Incomplete,
            (false, false) => PublisherState::Published,
        })
    }

    /// Whether the release already has a matching asset; a different or unverifiable one conflicts.
    fn remote_asset(
        &self,
        release: &Release,
        github_release: &GithubRelease,
        asset: &LocalAsset,
    ) -> Result<bool, String> {
        let Some(remote) = github_release
            .assets
            .iter()
            .find(|remote| remote.name == asset.name)
        else {
            return Ok(false);
        };
        match remote.digest.as_deref() {
            Some(digest) if digest == asset.digest => Ok(true),
            Some(digest) => Err(format!(
                "asset {} of GitHub Release {:?} in {} has digest {digest}, expected {}",
                asset.name, release.tag, self.repository, asset.digest
            )),
            None => Err(format!(
                "asset {} of GitHub Release {:?} in {} has no digest, so Presswerk cannot tell whether it matches",
                asset.name, release.tag, self.repository
            )),
        }
    }
}

impl Publisher for GithubPublisher {
    fn state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release) -> Result<ArtifactSteps, PublicationError> {
        let github_release = match self.client.release(&self.repository, &release.tag)? {
            None => self.client.create_draft(&self.repository, release)?,
            Some(existing) => existing,
        };
        let expected_prerelease = !release.version.pre.is_empty();
        if github_release.prerelease != expected_prerelease {
            return Err(PublicationError::new(format!(
                "GitHub Release {:?} in {} has prerelease={}, expected {expected_prerelease}",
                release.tag, self.repository, github_release.prerelease
            )));
        }
        // Assets go up before a draft is published, so a release never appears without them.
        let mut steps = Vec::with_capacity(self.assets.len());
        for asset in &self.assets {
            let step = if self
                .remote_asset(release, &github_release, asset)
                .map_err(PublicationError::new)?
            {
                AssetStep::AlreadyUploaded
            } else {
                self.client
                    .upload_asset(&self.repository, release, &github_release, asset)?;
                AssetStep::Uploaded
            };
            steps.push(ArtifactStep {
                kind: ArtifactKind::Asset,
                name: asset.name.clone(),
                step,
            });
        }
        if github_release.draft {
            self.client
                .publish_draft(&self.repository, release, github_release.id)?;
        }

        match self.checked_state(release)? {
            PublisherState::Published => Ok(steps),
            state => Err(PublicationError::new(format!(
                "GitHub Release {:?} in {} was not published: {state:?}",
                release.tag, self.repository
            ))),
        }
    }

    fn complete_steps(&self, _release: &Release) -> Result<ArtifactSteps, PublicationError> {
        Ok(self
            .assets
            .iter()
            .map(|asset| ArtifactStep {
                kind: ArtifactKind::Asset,
                name: asset.name.clone(),
                step: AssetStep::AlreadyUploaded,
            })
            .collect())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    id: u64,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    upload_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    /// The asset's content digest, such as `sha256:…`; GitHub reports it for uploaded assets.
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListedGithubRelease {
    tag_name: String,
    #[serde(flatten)]
    release: GithubRelease,
}

#[derive(Debug, Serialize)]
struct GithubReleaseRequest<'a> {
    tag_name: &'a str,
    target_commitish: &'a str,
    name: &'a str,
    body: &'a str,
    draft: bool,
    prerelease: bool,
}

#[derive(Debug, Clone)]
pub struct GitTagPublisher {
    repository: PathBuf,
    remote: String,
}

impl GitTagPublisher {
    pub fn new(repository: impl Into<PathBuf>, remote: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            remote: remote.into(),
        }
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
                "could not inspect Git tag {tag:?} on remote {:?} (git exited with {})",
                self.remote, output.status
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
        let repository = Repository::open(&self.repository).map_err(|error| {
            PublicationError::new(format!(
                "could not open Git repository {}: {error}",
                self.repository.display()
            ))
        })?;
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

        let signature = repository.signature().map_err(|error| {
            PublicationError::new(format!("could not read Git tagger configuration: {error}"))
        })?;
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
                "could not push Git tag {:?} to remote {:?} (git exited with {})",
                release.tag, self.remote, output.status
            ))),
        }
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
    /// One step per publisher, named like its declaration, such as `github-release.default`,
    /// with one step per artifact of that publisher.
    pub publisher_steps: Vec<(String, PublisherStep, ArtifactSteps)>,
}

impl PublishReport {
    /// Whether every step was already complete before this run.
    #[must_use]
    pub fn changed_nothing(&self) -> bool {
        self.tag_step == TagStep::AlreadyPushed
            && self
                .publisher_steps
                .iter()
                .all(|(_, step, _)| *step == PublisherStep::AlreadyPublished)
    }
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
) -> Result<PublishOutcome, PublicationError> {
    publish_to_remote(
        repository_path.as_ref(),
        revision,
        "origin",
        &GitCliffAnalyzer,
    )
}

pub fn publish_to_remote(
    repository_path: &Path,
    revision: &str,
    remote: &str,
    analyzer: &dyn Analyzer,
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

    let tag = declaration
        .tag
        .render(&declaration.version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    let changelog = read_text_at(repository_path, commit, &declaration.changelog.path)?;
    let notes = read_version_section(&changelog, &declaration.version)
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
        .oci
        .iter()
        .flat_map(|(name, publisher)| {
            publisher.images.iter().map(move |(platform, path)| {
                (format!("oci.{name} image {platform}"), path.as_str())
            })
        });
    crate::artifacts::fetch(images.chain(assets))
        .map_err(|error| PublicationError::new(error.to_string()))?;

    match tag_publisher.state(&release)? {
        RemoteState::Missing => {
            let parent_version =
                verify_candidate_predecessor(repository_path, commit, &declaration, remote, true)?;
            validate_candidate(
                repository_path,
                commit,
                &declaration,
                &parent_version,
                &changelog,
                analyzer,
            )?;
        }
        RemoteState::Matching => {
            verify_candidate_predecessor(repository_path, commit, &declaration, remote, false)?;
        }
        RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
    }

    // The forge release comes last, so it never announces a release whose images are missing.
    let mut publishers: Vec<Box<dyn Publisher>> = Vec::new();
    let mut publisher_names = Vec::new();
    for (name, publisher) in &declaration.publishers.oci {
        publishers.push(Box::new(crate::oci::OciPublisher::from_declaration(
            publisher,
        )?));
        publisher_names.push(format!("oci.{name}"));
    }
    for (name, publisher) in &declaration.publishers.github_release {
        publishers.push(Box::new(GithubPublisher::from_declaration(publisher)?));
        publisher_names.push(format!("github-release.{name}"));
    }
    let (tag_step, steps) = publish_ordered(&tag_publisher, &release, &publishers)?;
    Ok(PublishOutcome::Published(PublishReport {
        commit: release.commit,
        tag: release.tag,
        tag_step,
        publisher_steps: publisher_names
            .into_iter()
            .zip(steps)
            .map(|(name, (step, artifacts))| (name, step, artifacts))
            .collect(),
    }))
}

/// Publishes the tag, then completes every publisher, and reports what each step found.
pub fn publish_ordered(
    tag_publisher: &dyn TagPublisher,
    release: &Release,
    publishers: &[Box<dyn Publisher>],
) -> Result<(TagStep, Vec<(PublisherStep, ArtifactSteps)>), PublicationError> {
    if publishers.is_empty() {
        return Err(PublicationError::new(
            "at least one GitHub Release publisher is required",
        ));
    }
    let tag_step = match tag_publisher.state(release)? {
        RemoteState::Missing => TagStep::Pushed,
        RemoteState::Matching => TagStep::AlreadyPushed,
        RemoteState::Conflicting(message) => return Err(PublicationError::new(message)),
    };
    for publisher in publishers {
        if let PublisherState::Conflicting(message) = publisher.state(release)? {
            return Err(PublicationError::new(message));
        }
    }

    tag_publisher.publish(release)?;
    let mut publisher_steps = Vec::with_capacity(publishers.len());
    for publisher in publishers {
        let step = match publisher.state(release)? {
            PublisherState::Published => PublisherStep::AlreadyPublished,
            PublisherState::Missing => PublisherStep::Published,
            PublisherState::Draft => PublisherStep::DraftCompleted,
            PublisherState::Incomplete => PublisherStep::ArtifactsCompleted,
            PublisherState::Conflicting(message) => return Err(PublicationError::new(message)),
        };
        let artifacts = if step == PublisherStep::AlreadyPublished {
            publisher.complete_steps(release)?
        } else {
            publisher.publish(release)?
        };
        publisher_steps.push((step, artifacts));
    }

    match tag_publisher.state(release)? {
        RemoteState::Matching => {}
        state => {
            return Err(PublicationError::new(format!(
                "Git tag {:?} was not published: {state:?}",
                release.tag
            )));
        }
    }
    for publisher in publishers {
        match publisher.state(release)? {
            PublisherState::Published => {}
            state => {
                return Err(PublicationError::new(format!(
                    "publisher did not complete {:?}: {state:?}",
                    release.tag
                )));
            }
        }
    }
    Ok((tag_step, publisher_steps))
}

pub fn validate_new_release(
    version: &Version,
    parent_version: &Version,
    stable_base: Option<&Version>,
    analysis: &Analysis,
    changelog: &str,
) -> Result<(), PublicationError> {
    read_version_section(changelog, version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    if !is_valid_successor(parent_version, version) {
        return Err(PublicationError::new(format!(
            "release version {version} is not a valid successor of {parent_version}"
        )));
    }
    if let Some(minimum) = analysis
        .next_version(stable_base)
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

fn verify_candidate_predecessor(
    repository_path: &Path,
    commit: Oid,
    declaration: &ReleaseDeclaration,
    remote: &str,
    verify_completion: bool,
) -> Result<Version, PublicationError> {
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
    )?;
    Ok(parent_version)
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
    let stable_base = stable_base(&repository, commit, declaration)?;
    let range = stable_base
        .as_ref()
        .map_or_else(
            || CommitRange::initial(repository_path, commit.to_string()),
            |(_, base)| CommitRange::between(repository_path, base.to_string(), commit.to_string()),
        )
        .map_err(|error| PublicationError::new(error.to_string()))?
        .excluding(commit);
    let analysis = analyzer
        .analyze(&range, &declaration.analyzer.gitmoji.catalog)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    validate_new_release(
        &declaration.version,
        parent_version,
        stable_base.as_ref().map(|(version, _)| version),
        &analysis,
        changelog,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPredecessor {
    pub version: Version,
    pub tag: String,
    pub commit: Oid,
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
    )
}

fn verify_predecessor_with_trust_mode(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    remote: &str,
    trusted_predecessor: Option<&TrustedPredecessor>,
    verify_completion: bool,
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
            Err(error) => Err(PublicationError::new(error.to_string())),
        },
        |publisher, tag, _version| {
            GithubClient::from_environment(&publisher.api_url, &publisher.token.secret)?
                .release_state(&publisher.repository, tag)
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn verify_predecessor_with<E, F>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    trusted_predecessor: Option<&TrustedPredecessor>,
    evaluate_declaration: E,
    release_state: F,
) -> Result<Option<VerifiedPredecessor>, PublicationError>
where
    E: FnMut(Oid) -> Result<ReleaseDeclaration, PublicationError>,
    F: FnMut(
        &GithubReleaseDeclaration,
        &str,
        &Version,
    ) -> Result<GithubReleaseState, PublicationError>,
{
    verify_predecessor_with_mode(
        repository_path,
        candidate,
        previous_version,
        current_declaration,
        tag_publisher,
        trusted_predecessor,
        true,
        evaluate_declaration,
        release_state,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_predecessor_with_mode<E, F>(
    repository_path: &Path,
    candidate: Oid,
    previous_version: &Version,
    current_declaration: &ReleaseDeclaration,
    tag_publisher: &dyn TagPublisher,
    trusted_predecessor: Option<&TrustedPredecessor>,
    verify_completion: bool,
    mut evaluate_declaration: E,
    mut release_state: F,
) -> Result<Option<VerifiedPredecessor>, PublicationError>
where
    E: FnMut(Oid) -> Result<ReleaseDeclaration, PublicationError>,
    F: FnMut(
        &GithubReleaseDeclaration,
        &str,
        &Version,
    ) -> Result<GithubReleaseState, PublicationError>,
{
    require_github_publishers(current_declaration)?;
    let repository = Repository::open(repository_path).map_err(|error| {
        PublicationError::new(format!("could not open Git repository: {error}"))
    })?;
    let previous_tag = current_declaration
        .tag
        .render(previous_version)
        .map_err(|error| PublicationError::new(error.to_string()))?;
    let Some(previous_commit) = local_tagged_commit(&repository, &previous_tag)? else {
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
                    "trusted predecessor {previous_tag}={previous_commit} is unnecessary because its declaration schema is supported"
                )));
            }
            declaration
        }
        Err(error) => {
            let Some(unsupported) = error.as_unsupported_predecessor() else {
                return Err(error);
            };
            if unsupported.tag != previous_tag || unsupported.commit != previous_commit {
                return Err(error);
            }
            let Some(trusted) = trusted_predecessor else {
                return Err(error);
            };
            if trusted.tag != previous_tag || trusted.commit != previous_commit {
                return Err(PublicationError::new(format!(
                    "trusted predecessor {}={} does not match expected unsupported predecessor {previous_tag}={previous_commit}",
                    trusted.tag, trusted.commit
                )));
            }
            return verified_remote_predecessor(
                tag_publisher,
                previous_version,
                previous_tag,
                previous_commit,
            )
            .map(Some);
        }
    };
    require_github_publishers(&previous_declaration)?;
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
        }));
    }

    let verified = verified_remote_predecessor(
        tag_publisher,
        previous_version,
        previous_tag.clone(),
        previous_commit,
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
    Ok(Some(verified))
}

fn verified_remote_predecessor(
    tag_publisher: &dyn TagPublisher,
    previous_version: &Version,
    previous_tag: String,
    previous_commit: Oid,
) -> Result<VerifiedPredecessor, PublicationError> {
    let previous = Release {
        commit: previous_commit.to_string(),
        version: previous_version.clone(),
        tag: previous_tag.clone(),
        notes: String::new(),
    };
    match tag_publisher.state(&previous)? {
        RemoteState::Matching => Ok(VerifiedPredecessor {
            version: previous_version.clone(),
            tag: previous_tag,
            commit: previous_commit,
        }),
        RemoteState::Missing => Err(PublicationError::new(format!(
            "previous release tag {previous_tag:?} is missing from the remote"
        ))),
        RemoteState::Conflicting(message) => Err(PublicationError::new(message)),
    }
}

fn require_github_publishers(declaration: &ReleaseDeclaration) -> Result<(), PublicationError> {
    if declaration.publishers.github_release.is_empty() {
        return Err(PublicationError::new(
            "at least one GitHub Release publisher is required",
        ));
    }
    Ok(())
}

fn stable_base(
    repository: &Repository,
    candidate: Oid,
    declaration: &ReleaseDeclaration,
) -> Result<Option<(Version, Oid)>, PublicationError> {
    let names = repository
        .tag_names(None)
        .map_err(|error| PublicationError::new(format!("could not list Git tags: {error}")))?;
    let mut stable = None;
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
        if stable
            .as_ref()
            .is_none_or(|(current, _): &(Version, Oid)| version > *current)
        {
            stable = Some((version, commit));
        }
    }
    Ok(stable)
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
