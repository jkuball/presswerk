//! The GitHub Release publisher: creates a release for the tag, uploads its assets into the
//! draft, and publishes it.

use crate::declaration::{DeclarationError, validate_template};
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::shared::secrets::Credential;
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, shared,
};
use minijinja::{Environment, UndefinedBehavior, context};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;
use url::Url;

pub const DEFAULT_GITHUB_RELEASE_BODY: &str = "{{ notes }}";

/// A file attached to a GitHub Release.
pub const ASSET: ArtifactKind = ArtifactKind::new("asset", "uploaded");

/// `publishers.github-release.<name>`: a GitHub Release for the release tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubReleaseDeclaration {
    pub api_url: String,
    pub repository: String,
    pub token: Credential,
    #[serde(default = "default_github_release_body")]
    pub body: String,
    /// Files attached to the release, by asset name. Each is a path in the Nix store.
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

fn default_github_release_body() -> String {
    DEFAULT_GITHUB_RELEASE_BODY.into()
}

impl GithubReleaseDeclaration {
    fn validate(&self) -> Result<(), DeclarationError> {
        let mut repository_parts = self.repository.split('/');
        if repository_parts.next().is_none_or(str::is_empty)
            || repository_parts.next().is_none_or(str::is_empty)
            || repository_parts.next().is_some()
        {
            return Err(DeclarationError::Invalid(format!(
                "GitHub repository {:?} must use owner/name form",
                self.repository
            )));
        }
        let api_url = Url::parse(&self.api_url).map_err(|error| {
            DeclarationError::Invalid(format!(
                "invalid GitHub API URL {:?}: {error}",
                self.api_url
            ))
        })?;
        if !matches!(api_url.scheme(), "http" | "https")
            || api_url.cannot_be_a_base()
            || api_url.query().is_some()
            || api_url.fragment().is_some()
        {
            return Err(DeclarationError::Invalid(format!(
                "GitHub API URL {:?} must be an HTTP(S) base URL",
                self.api_url
            )));
        }
        validate_template(
            &self.body,
            "publishers.github-release.<name>.body",
            serde_json::json!({
                "notes": "### Features\n\n- Add search\n",
                "version": "1.2.3",
                "tag": "v1.2.3"
            }),
        )?;
        for (name, path) in &self.assets {
            if name.is_empty() || name.contains('/') {
                return Err(DeclarationError::Invalid(format!(
                    "GitHub Release asset name {name:?} must be non-empty and contain no slash"
                )));
            }
            if !path.starts_with('/') {
                return Err(DeclarationError::Invalid(format!(
                    "GitHub Release asset {name} must be an absolute store path, not {path:?}"
                )));
            }
        }
        self.token.validate_secret("GitHub token")
    }

    fn client(&self, ctx: &Context) -> Result<GithubClient, PublicationError> {
        GithubClient::new(&self.api_url, ctx.session.secrets.get(&self.token)?)
    }
}

/// The `github-release` publisher type.
#[derive(Debug)]
pub struct GithubRelease {
    declaration: GithubReleaseDeclaration,
    /// Built on first use, once the assets are fetched and the token is known.
    runner: OnceLock<GithubPublisher>,
}

impl GithubRelease {
    #[must_use]
    pub fn declaration(&self) -> &GithubReleaseDeclaration {
        &self.declaration
    }

    fn runner(&self, ctx: &Context) -> Result<&GithubPublisher, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let assets = self
            .declaration
            .assets
            .iter()
            .map(|(name, path)| LocalAsset::from_file(name, path))
            .collect::<Result<_, _>>()?;
        let runner =
            GithubPublisher::new(self.declaration.client(ctx)?, &self.declaration.repository)
                .with_body(&self.declaration.body)
                .with_changelog(&ctx.session.changelog)
                .with_assets(assets);
        Ok(self.runner.get_or_init(|| runner))
    }
}

/// Whether a GitHub Release in `state` completes `release`: published, with the expected
/// prerelease flag. Its assets are not checked; a republish repairs a missing one.
#[must_use]
pub fn completeness_of(
    repository: &str,
    release: &Release,
    state: &GithubReleaseState,
) -> Completeness {
    let expected_prerelease = !release.version.pre.is_empty();
    match state {
        GithubReleaseState::Published { prerelease } if *prerelease == expected_prerelease => {
            Completeness::Complete
        }
        state => Completeness::Incomplete {
            target: format!("GitHub Release {:?} in {repository}", release.tag),
            reason: format!("{state:?}"),
        },
    }
}

impl PublisherType for GithubRelease {
    const NAME: &'static str = "github-release";

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: GithubReleaseDeclaration = shared::decode(declaration)?;
        declaration.validate()?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

impl Publisher for GithubRelease {
    fn describe(&self, _version: &Version, tag: &str) -> Result<Description, String> {
        Ok(Description {
            target: self.declaration.repository.clone(),
            // Store paths are left out: the release commit's artifacts get their own paths.
            items: std::iter::once(("release".into(), tag.into()))
                .chain(
                    self.declaration
                        .assets
                        .keys()
                        .map(|asset| ("asset".into(), asset.clone())),
                )
                .collect(),
        })
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        self.declaration
            .assets
            .iter()
            .map(|(name, path)| (format!("asset {name}"), path.clone()))
            .collect()
    }

    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        let state = self
            .declaration
            .client(ctx)?
            .release_state(&self.declaration.repository, &release.tag)?;
        Ok(completeness_of(
            &self.declaration.repository,
            release,
            &state,
        ))
    }

    fn state(&self, release: &Release, ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.runner(ctx)?.state(release, ctx)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        self.runner(ctx)?.publish(release, ctx)
    }

    fn complete_steps(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        self.runner(ctx)?.complete_steps(release, ctx)
    }

    fn labels(&self) -> Labels {
        GithubPublisher::LABELS
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
    ) -> Result<Option<RemoteRelease>, PublicationError> {
        self.validate_repository(repository)?;
        let url = self.endpoint(repository, &["releases", "tags", tag])?;
        match self.request("GET", &url).call() {
            Ok(response) => response
                .into_json::<RemoteRelease>()
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
    ) -> Result<Option<RemoteRelease>, PublicationError> {
        const PER_PAGE: usize = 100;
        let mut matches = Vec::new();
        for page in 1.. {
            let mut url = self.endpoint(repository, &["releases"])?;
            url.query_pairs_mut()
                .append_pair("per_page", &PER_PAGE.to_string())
                .append_pair("page", &page.to_string());
            let releases: Vec<ListedRemoteRelease> = match self.request("GET", &url).call() {
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
    ) -> Result<RemoteRelease, PublicationError> {
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
                let created: RemoteRelease = response.into_json().map_err(|error| {
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
    ) -> Result<RemoteRelease, PublicationError> {
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
        github_release: &RemoteRelease,
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
        let digest = shared::digest::sha256_digest(&bytes);
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
    const LABELS: Labels = Labels {
        publisher: "release",
        artifacts: "assets",
    };

    pub fn new(client: GithubClient, repository: impl Into<String>) -> Self {
        Self {
            client,
            repository: repository.into(),
            body: DEFAULT_GITHUB_RELEASE_BODY.into(),
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
        github_release: &RemoteRelease,
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

/// A GitHub Release publisher with its own client, such as one for a test server.
impl Publisher for GithubPublisher {
    fn describe(&self, _version: &Version, tag: &str) -> Result<Description, String> {
        Ok(Description {
            target: self.repository.clone(),
            items: std::iter::once(("release".into(), tag.into()))
                .chain(
                    self.assets
                        .iter()
                        .map(|asset| ("asset".into(), asset.name.clone())),
                )
                .collect(),
        })
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        let state = self.client.release_state(&self.repository, &release.tag)?;
        Ok(completeness_of(&self.repository, release, &state))
    }

    fn state(&self, release: &Release, _ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
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
                kind: ASSET,
                name: asset.name.clone(),
                step,
            };
            ctx.progress(&step);
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

    fn complete_steps(
        &self,
        _release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        Ok(self
            .assets
            .iter()
            .map(|asset| ArtifactStep {
                kind: ASSET,
                name: asset.name.clone(),
                step: AssetStep::AlreadyUploaded,
            })
            .collect())
    }

    fn labels(&self) -> Labels {
        Self::LABELS
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRelease {
    id: u64,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    #[serde(default)]
    upload_url: Option<String>,
    #[serde(default)]
    assets: Vec<RemoteAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteAsset {
    name: String,
    /// The asset's content digest, such as `sha256:…`; GitHub reports it for uploaded assets.
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListedRemoteRelease {
    tag_name: String,
    #[serde(flatten)]
    release: RemoteRelease,
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
