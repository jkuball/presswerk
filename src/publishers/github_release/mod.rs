//! The GitHub Release publisher: creates a release for the tag, uploads its assets into the
//! draft, and publishes it.

use crate::declaration::GithubReleasePublisher as GithubReleaseDeclaration;
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, Publisher,
    PublisherState, Release,
};
use minijinja::{Environment, UndefinedBehavior, context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fmt;
use std::path::PathBuf;
use url::Url;

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

    /// The repository's web URL, e.g. `https://github.com/owner/project`.
    fn repository_url(&self, repository: &str) -> Result<Url, PublicationError> {
        #[derive(Deserialize)]
        struct GithubRepository {
            html_url: String,
        }
        let url = self.endpoint(repository, &[])?;
        match self.request("GET", &url).call() {
            Ok(response) => response
                .into_json::<GithubRepository>()
                .map_err(|error| error.to_string())
                .and_then(|github| Url::parse(&github.html_url).map_err(|error| error.to_string()))
                .map_err(|error| {
                    PublicationError::new(format!(
                        "could not decode GitHub repository {repository:?}: {error}"
                    ))
                }),
            Err(ureq::Error::Status(status, _)) => Err(PublicationError::new(format!(
                "GitHub repository {repository:?} is inaccessible (HTTP {status})"
            ))),
            Err(ureq::Error::Transport(error)) => Err(PublicationError::new(format!(
                "could not inspect GitHub repository {repository:?}: {error}"
            ))),
        }
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
        body: &str,
    ) -> Result<GithubRelease, PublicationError> {
        let url = self.endpoint(repository, &["releases"])?;
        let request = GithubReleaseRequest {
            tag_name: &release.tag,
            target_commitish: &release.commit,
            name: &release.tag,
            body,
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
    body: String,
    changelog: String,
    assets: Vec<LocalAsset>,
}

/// GitHub rejects release bodies longer than this many characters.
pub const GITHUB_RELEASE_BODY_LIMIT: usize = 125_000;

impl GithubPublisher {
    /// Build the publisher from its declaration; its assets must already be fetched.
    pub fn from_declaration(
        declaration: &GithubReleaseDeclaration,
        changelog: &str,
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
            body: declaration.body.clone(),
            changelog: changelog.into(),
            assets,
        })
    }

    pub fn new(client: GithubClient, repository: impl Into<String>) -> Self {
        Self {
            client,
            repository: repository.into(),
            body: crate::declaration::DEFAULT_GITHUB_RELEASE_BODY.into(),
            changelog: "CHANGELOG.md".into(),
            assets: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_changelog(mut self, changelog: impl Into<String>) -> Self {
        self.changelog = changelog.into();
        self
    }

    #[must_use]
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    #[must_use]
    pub fn with_assets(mut self, assets: Vec<LocalAsset>) -> Self {
        self.assets = assets;
        self
    }

    fn render_body(&self, release: &Release) -> Result<String, PublicationError> {
        let mut environment = Environment::new();
        environment.set_undefined_behavior(UndefinedBehavior::Strict);
        environment
            .render_str(
                &self.body,
                context!(
                    notes => &release.notes,
                    version => release.version.to_string(),
                    tag => &release.tag
                ),
            )
            .map_err(|error| {
                PublicationError::new(format!(
                    "failed to render GitHub Release body template: {error}"
                ))
            })
    }

    /// The rendered body, or a link to the changelog at the tag when GitHub would reject it.
    fn body(&self, release: &Release) -> Result<String, PublicationError> {
        let body = self.render_body(release)?;
        if body.chars().count() <= GITHUB_RELEASE_BODY_LIMIT {
            return Ok(body);
        }
        let mut changelog = self.client.repository_url(&self.repository)?;
        changelog
            .path_segments_mut()
            .map_err(|()| {
                PublicationError::new(format!(
                    "GitHub repository URL of {} cannot contain path segments",
                    self.repository
                ))
            })?
            .pop_if_empty()
            .push("blob")
            .push(&release.tag)
            .extend(self.changelog.split('/'));
        Ok(format!(
            "The release notes are too long for a GitHub Release; read them in [{}]({changelog}).",
            self.changelog
        ))
    }

    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        let body = self.body(release)?;
        let Some(github_release) = self.client.release(&self.repository, &release.tag)? else {
            return Ok(PublisherState::Missing);
        };
        if github_release.draft && github_release.body.as_deref() != Some(body.as_str()) {
            return Ok(PublisherState::Conflicting(format!(
                "draft GitHub Release {:?} in {} has a body that does not match the declared template",
                release.tag, self.repository
            )));
        }
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

    fn publish_with_progress(
        &self,
        release: &Release,
        progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        let body = self.body(release)?;
        let github_release = match self.client.release(&self.repository, &release.tag)? {
            None => self.client.create_draft(&self.repository, release, &body)?,
            Some(existing) => existing,
        };
        if github_release.draft && github_release.body.as_deref() != Some(body.as_str()) {
            return Err(PublicationError::new(format!(
                "draft GitHub Release {:?} in {} has a body that does not match the declared template",
                release.tag, self.repository
            )));
        }
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
            let step = ArtifactStep {
                kind: ArtifactKind::Asset,
                name: asset.name.clone(),
                step,
            };
            progress(&step);
            steps.push(step);
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
    body: Option<String>,
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
