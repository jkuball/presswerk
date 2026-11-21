//! The Python publisher: uploads the wheels and the sdist of a release to a package index, such
//! as PyPI, through the legacy upload API, and reads its state from the simple API.

pub mod distribution;
pub mod index;

use crate::declaration::DeclarationError;
use crate::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, PublicationError, PublisherState, Release,
};
use crate::publishers::shared::secrets::Credential;
use crate::publishers::{
    Completeness, Context, Description, Labels, Publisher, PublisherType, shared,
};
use distribution::{Distribution, Filename, normalize};
use index::{HttpIndex, Index, IndexFile, Login};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use url::Url;

/// One wheel or sdist on the index.
pub const FILE: ArtifactKind = ArtifactKind::new("file", "uploaded");

const LABELS: Labels = Labels {
    publisher: "index",
    artifacts: "files",
};

/// `publishers.python.<name>`: the distributions of one project on one package index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonDeclaration {
    /// The project name on the index, such as `my-package`.
    pub name: String,
    /// The store path of a directory that holds the wheels and the sdist.
    pub distributions: String,
    /// The legacy upload API, such as `https://upload.pypi.org/legacy/`.
    pub repository_url: String,
    /// The simple API, such as `https://pypi.org/simple/`.
    pub index_url: String,
    pub username: Credential,
    pub password: Credential,
}

impl PythonDeclaration {
    fn validate(&self) -> Result<(), DeclarationError> {
        if !is_project_name(&self.name) {
            return Err(DeclarationError::Invalid(format!(
                "Python project name {:?} must start and end with a letter or digit and contain only letters, digits, `-`, `_`, and `.`",
                self.name
            )));
        }
        if !self.distributions.starts_with('/') {
            return Err(DeclarationError::Invalid(format!(
                "Python distributions must be an absolute store path, not {:?}",
                self.distributions
            )));
        }
        validate_url("repositoryUrl", &self.repository_url)?;
        validate_url("indexUrl", &self.index_url)?;
        self.username.validate()?;
        self.password.validate_secret("index password")
    }

    /// The index with whatever credentials are set: reads may do without them.
    fn index(&self, ctx: &Context) -> Result<HttpIndex, PublicationError> {
        let secrets = &ctx.session.secrets;
        let login = if secrets.is_set(&self.username) && secrets.is_set(&self.password) {
            Some(Login {
                username: secrets.get(&self.username)?,
                password: secrets.get(&self.password)?,
            })
        } else {
            None
        };
        HttpIndex::new(&self.repository_url, &self.index_url, login)
    }
}

/// A project name as [PEP 508] allows it.
///
/// [PEP 508]: https://peps.python.org/pep-0508/#names
fn is_project_name(name: &str) -> bool {
    let alphanumeric =
        |character: Option<char>| character.is_some_and(|c| c.is_ascii_alphanumeric());
    alphanumeric(name.chars().next())
        && alphanumeric(name.chars().last())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn validate_url(option: &str, url: &str) -> Result<(), DeclarationError> {
    let parsed = Url::parse(url)
        .map_err(|error| DeclarationError::Invalid(format!("invalid {option} {url:?}: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.cannot_be_a_base()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(DeclarationError::Invalid(format!(
            "{option} {url:?} must be an HTTP(S) URL without credentials, query, or fragment"
        )));
    }
    Ok(())
}

/// The `python` publisher type.
#[derive(Debug)]
pub struct Python {
    declaration: PythonDeclaration,
    /// Built on first use, once the distributions are fetched and the credentials are known.
    runner: OnceLock<PythonPublisher<HttpIndex>>,
}

impl Python {
    #[must_use]
    pub fn declaration(&self) -> &PythonDeclaration {
        &self.declaration
    }

    fn runner(&self, ctx: &Context) -> Result<&PythonPublisher<HttpIndex>, PublicationError> {
        if let Some(runner) = self.runner.get() {
            return Ok(runner);
        }
        let runner = PythonPublisher::new(
            self.declaration.index(ctx)?,
            &self.declaration.name,
            &self.declaration.index_url,
            distribution::read_directory(Path::new(&self.declaration.distributions))?,
        );
        Ok(self.runner.get_or_init(|| runner))
    }
}

impl PublisherType for Python {
    const NAME: &'static str = "python";

    fn from_declaration(declaration: &Value) -> Result<Self, DeclarationError> {
        let declaration: PythonDeclaration = shared::decode(declaration)?;
        declaration.validate()?;
        Ok(Self {
            declaration,
            runner: OnceLock::new(),
        })
    }
}

fn describe(project: &str, index_url: &str, version: &Version) -> Result<Description, String> {
    // A pre-release's SemVer and PEP 440 spellings differ, such as `1.4.0-rc.1` and `1.4.0rc1`.
    if !version.pre.is_empty() {
        return Err(format!(
            "Python publishers cannot release the pre-release version {version} yet"
        ));
    }
    Ok(Description {
        target: index_url.to_owned(),
        items: vec![("release".into(), format!("{project}=={version}"))],
    })
}

/// Whether the index lists a file of `release` for `project`. Without the local distributions it
/// cannot tell which files belong to the release, so one is enough; a publish completes the rest.
pub fn completeness_of(
    index: &impl Index,
    project: &str,
    index_url: &str,
    release: &Release,
) -> Completeness {
    let version = release.version.to_string();
    match index.files(project) {
        Ok(files)
            if files
                .iter()
                .any(|file| is_release_file(file, project, &version)) =>
        {
            Completeness::Complete
        }
        Ok(_) => Completeness::Incomplete {
            target: format!("{project}=={version} in {index_url}"),
            reason: "the index lists no file of this version".into(),
        },
        Err(error) => Completeness::Unknown(error.to_string()),
    }
}

fn is_release_file(file: &IndexFile, project: &str, version: &str) -> bool {
    Filename::parse(&file.filename).is_some_and(|parsed| {
        normalize(&parsed.name) == normalize(project) && parsed.version == version
    })
}

impl Publisher for Python {
    fn credentials(&self) -> Vec<(&'static str, &Credential)> {
        vec![
            ("username", &self.declaration.username),
            ("password", &self.declaration.password),
        ]
    }

    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.declaration.name, &self.declaration.index_url, version)
    }

    fn artifacts(&self) -> Vec<(String, String)> {
        vec![(
            "distributions".into(),
            self.declaration.distributions.clone(),
        )]
    }

    fn completeness(
        &self,
        release: &Release,
        ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        Ok(completeness_of(
            &self.declaration.index(ctx)?,
            &self.declaration.name,
            &self.declaration.index_url,
            release,
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
        LABELS
    }
}

/// How long a publish waits for the simple API to list the files it uploaded.
#[derive(Debug, Clone, Copy)]
pub struct Lag {
    pub timeout: Duration,
    pub interval: Duration,
}

impl Default for Lag {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            interval: Duration::from_secs(3),
        }
    }
}

/// What the index holds of one local file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileState {
    Missing,
    Matching,
    Conflicting(String),
}

/// A Python publisher with its own index, such as a fake one in tests.
#[derive(Debug)]
pub struct PythonPublisher<I> {
    index: I,
    project: String,
    index_url: String,
    distributions: Vec<Distribution>,
    lag: Lag,
}

impl<I: Index> PythonPublisher<I> {
    pub fn new(
        index: I,
        project: impl Into<String>,
        index_url: impl Into<String>,
        distributions: Vec<Distribution>,
    ) -> Self {
        Self {
            index,
            project: project.into(),
            index_url: index_url.into(),
            distributions,
            lag: Lag::default(),
        }
    }

    #[must_use]
    pub fn with_lag(mut self, lag: Lag) -> Self {
        self.lag = lag;
        self
    }

    /// Compares each file's project name and version with the release; nothing else is checked.
    fn check_versions(&self, release: &Release) -> Result<(), PublicationError> {
        let version = release.version.to_string();
        let project = normalize(&self.project);
        for distribution in &self.distributions {
            let mismatch = |what: &str, found: &str, expected: &str| {
                Err(PublicationError::new(format!(
                    "{} has {what} {found:?}, expected {expected:?}",
                    distribution.filename
                )))
            };
            if normalize(&distribution.parsed.name) != project {
                return mismatch("the project name", &distribution.parsed.name, &self.project);
            }
            if distribution.parsed.version != version {
                return mismatch("the version", &distribution.parsed.version, &version);
            }
            match distribution.field("name") {
                Some(name) if normalize(name) == project => {}
                found => {
                    return mismatch("the metadata name", found.unwrap_or(""), &self.project);
                }
            }
            match distribution.field("version") {
                Some(found) if found == version => {}
                found => return mismatch("the metadata version", found.unwrap_or(""), &version),
            }
        }
        Ok(())
    }

    /// What the index holds of one local file. A listed file without a SHA-256 digest cannot be
    /// verified, which is an error rather than a conflict: nothing says its bytes differ.
    fn file_state(
        &self,
        distribution: &Distribution,
        remote: &[IndexFile],
    ) -> Result<FileState, PublicationError> {
        let Some(listed) = remote
            .iter()
            .find(|file| file.filename == distribution.filename)
        else {
            return Ok(FileState::Missing);
        };
        match &listed.sha256 {
            Some(digest) if *digest == distribution.sha256 => Ok(FileState::Matching),
            Some(digest) => Ok(FileState::Conflicting(format!(
                "{} on {} has sha256 {digest}, expected {}",
                distribution.filename, self.index_url, distribution.sha256
            ))),
            None => Err(PublicationError::new(format!(
                "{} on {} has no sha256 digest, so Presswerk cannot verify whether it matches",
                distribution.filename, self.index_url
            ))),
        }
    }

    /// The state of every local file in one listing of the index.
    fn file_states(&self, remote: &[IndexFile]) -> Result<Vec<FileState>, PublicationError> {
        self.distributions
            .iter()
            .map(|distribution| self.file_state(distribution, remote))
            .collect()
    }

    fn checked_state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
        self.check_versions(release)?;
        let states = self.file_states(&self.index.files(&self.project)?)?;
        if let Some(FileState::Conflicting(conflict)) = states
            .iter()
            .find(|state| matches!(state, FileState::Conflicting(_)))
        {
            return Ok(PublisherState::Conflicting(conflict.clone()));
        }
        Ok(
            if states.iter().all(|state| *state == FileState::Matching) {
                PublisherState::Published
            } else if states.iter().all(|state| *state == FileState::Missing) {
                PublisherState::Missing
            } else {
                PublisherState::Incomplete
            },
        )
    }

    /// Waits until the simple API lists every file in `uploaded`, as an index may list a new
    /// file only some time after its upload.
    fn wait_until_listed(&self, uploaded: &[&Distribution]) -> Result<(), PublicationError> {
        if uploaded.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + self.lag.timeout;
        loop {
            let remote = self.index.files_before(&self.project, deadline)?;
            let mut unlisted = Vec::new();
            for distribution in uploaded {
                match self.file_state(distribution, &remote)? {
                    FileState::Matching => {}
                    FileState::Missing => unlisted.push(distribution.filename.as_str()),
                    FileState::Conflicting(conflict) => {
                        return Err(PublicationError::new(conflict));
                    }
                }
            }
            if unlisted.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(PublicationError::new(format!(
                    "{} does not list {} {} seconds after the upload; publish again once it does",
                    self.index_url,
                    unlisted.join(", "),
                    self.lag.timeout.as_secs()
                )));
            }
            std::thread::sleep(self.lag.interval);
        }
    }
}

impl<I: Index> Publisher for PythonPublisher<I> {
    fn describe(&self, version: &Version, _tag: &str) -> Result<Description, String> {
        describe(&self.project, &self.index_url, version)
    }

    fn completeness(
        &self,
        release: &Release,
        _ctx: &Context,
    ) -> Result<Completeness, PublicationError> {
        Ok(completeness_of(
            &self.index,
            &self.project,
            &self.index_url,
            release,
        ))
    }

    fn state(&self, release: &Release, _ctx: &Context) -> Result<PublisherState, PublicationError> {
        self.checked_state(release)
    }

    fn publish(&self, release: &Release, ctx: &Context) -> Result<ArtifactSteps, PublicationError> {
        self.check_versions(release)?;
        // Every file of this listing is checked before the first upload, so a conflict that
        // appeared since the state check stops the run before anything is written.
        let states = self.file_states(&self.index.files(&self.project)?)?;
        if let Some(FileState::Conflicting(conflict)) = states
            .iter()
            .find(|state| matches!(state, FileState::Conflicting(_)))
        {
            return Err(PublicationError::new(conflict.clone()));
        }
        let mut steps = Vec::with_capacity(self.distributions.len());
        let mut uploaded = Vec::new();
        for (distribution, state) in self.distributions.iter().zip(states) {
            let step = match state {
                FileState::Matching => AssetStep::AlreadyUploaded,
                FileState::Conflicting(_) => unreachable!("conflicts stopped the run"),
                FileState::Missing => match self.index.upload(distribution) {
                    Ok(()) => {
                        uploaded.push(distribution);
                        AssetStep::Uploaded
                    }
                    // A concurrent run may have uploaded the same file; accept it only if it
                    // matches.
                    Err(error) => {
                        let remote = self.index.files(&self.project)?;
                        if !matches!(
                            self.file_state(distribution, &remote),
                            Ok(FileState::Matching)
                        ) {
                            return Err(error);
                        }
                        AssetStep::AlreadyUploaded
                    }
                },
            };
            let step = ArtifactStep {
                kind: FILE,
                name: distribution.filename.clone(),
                step,
            };
            ctx.progress(&step);
            steps.push(step);
        }
        self.wait_until_listed(&uploaded)?;
        match self.checked_state(release)? {
            PublisherState::Published => Ok(steps),
            state => Err(PublicationError::new(format!(
                "{}=={} on {} was not published: {state:?}",
                self.project, release.version, self.index_url
            ))),
        }
    }

    fn complete_steps(
        &self,
        _release: &Release,
        _ctx: &Context,
    ) -> Result<ArtifactSteps, PublicationError> {
        Ok(self
            .distributions
            .iter()
            .map(|distribution| ArtifactStep {
                kind: FILE,
                name: distribution.filename.clone(),
                step: AssetStep::AlreadyUploaded,
            })
            .collect())
    }

    fn labels(&self) -> Labels {
        LABELS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distribution::tests::{METADATA, sdist, wheel};
    use serde_json::json;
    use std::sync::Mutex;

    const WHEEL: &str = "my_package-1.2.3-py3-none-any.whl";
    const SDIST: &str = "my_package-1.2.3.tar.gz";

    /// An index in memory. Each upload becomes visible only after `lag` more reads.
    #[derive(Default)]
    struct FakeIndex {
        files: Mutex<Vec<(IndexFile, usize)>>,
        uploads: Mutex<Vec<String>>,
        lag: usize,
        failing_uploads: Vec<String>,
    }

    impl FakeIndex {
        fn with(files: &[(&str, Option<&str>)]) -> Self {
            Self {
                files: Mutex::new(
                    files
                        .iter()
                        .map(|(filename, sha256)| {
                            (
                                IndexFile {
                                    filename: (*filename).into(),
                                    sha256: sha256.map(Into::into),
                                },
                                0,
                            )
                        })
                        .collect(),
                ),
                ..Self::default()
            }
        }
    }

    impl Index for FakeIndex {
        fn files(&self, project: &str) -> Result<Vec<IndexFile>, PublicationError> {
            assert_eq!(project, "My.Package");
            let mut files = self.files.lock().unwrap();
            let listed = files
                .iter()
                .filter(|(_, hidden)| *hidden == 0)
                .map(|(file, _)| file.clone())
                .collect();
            for (_, hidden) in files.iter_mut() {
                *hidden = hidden.saturating_sub(1);
            }
            Ok(listed)
        }

        fn upload(&self, distribution: &Distribution) -> Result<(), PublicationError> {
            if self.failing_uploads.contains(&distribution.filename) {
                return Err(PublicationError::new("HTTP 400 File already exists"));
            }
            self.uploads
                .lock()
                .unwrap()
                .push(distribution.filename.clone());
            self.files.lock().unwrap().push((
                IndexFile {
                    filename: distribution.filename.clone(),
                    sha256: Some(distribution.sha256.clone()),
                },
                self.lag,
            ));
            Ok(())
        }
    }

    fn release(version: &str) -> Release {
        Release {
            commit: "0123456789012345678901234567890123456789".into(),
            version: Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            notes: String::new(),
        }
    }

    fn distributions(directory: &Path) -> Vec<Distribution> {
        wheel(directory, WHEEL, METADATA);
        sdist(directory, SDIST, METADATA);
        distribution::read_directory(directory).unwrap()
    }

    fn publisher(index: FakeIndex, distributions: Vec<Distribution>) -> PythonPublisher<FakeIndex> {
        PythonPublisher::new(
            index,
            "My.Package",
            "https://pypi.org/simple/",
            distributions,
        )
        .with_lag(Lag {
            timeout: Duration::from_secs(5),
            interval: Duration::ZERO,
        })
    }

    fn steps(steps: &ArtifactSteps) -> Vec<(&str, &AssetStep)> {
        steps
            .iter()
            .map(|step| (step.name.as_str(), &step.step))
            .collect()
    }

    #[test]
    fn uploads_every_file_and_a_retry_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = publisher(
            FakeIndex {
                lag: 2,
                ..FakeIndex::default()
            },
            distributions(directory.path()),
        );
        let release = release("1.2.3");
        let ctx = Context::detached();

        assert_eq!(
            publisher.state(&release, &ctx).unwrap(),
            PublisherState::Missing
        );
        assert!(matches!(
            publisher.completeness(&release, &ctx).unwrap(),
            Completeness::Incomplete { .. }
        ));
        let first = publisher.publish(&release, &ctx).unwrap();
        assert_eq!(
            steps(&first),
            [(WHEEL, &AssetStep::Uploaded), (SDIST, &AssetStep::Uploaded)]
        );
        assert_eq!(*publisher.index.uploads.lock().unwrap(), [WHEEL, SDIST]);

        assert_eq!(
            publisher.state(&release, &ctx).unwrap(),
            PublisherState::Published
        );
        assert_eq!(
            publisher.completeness(&release, &ctx).unwrap(),
            Completeness::Complete
        );
        let retry = publisher.publish(&release, &ctx).unwrap();
        assert_eq!(
            steps(&retry),
            [
                (WHEEL, &AssetStep::AlreadyUploaded),
                (SDIST, &AssetStep::AlreadyUploaded)
            ]
        );
        assert_eq!(publisher.index.uploads.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_partial_release_is_completed() {
        let directory = tempfile::tempdir().unwrap();
        let distributions = distributions(directory.path());
        let sdist = distributions[1].sha256.clone();
        let publisher = publisher(FakeIndex::with(&[(SDIST, Some(&sdist))]), distributions);
        let release = release("1.2.3");
        let ctx = Context::detached();

        assert_eq!(
            publisher.state(&release, &ctx).unwrap(),
            PublisherState::Incomplete
        );
        // The previous release counts as complete: without its files, one is enough.
        assert_eq!(
            publisher.completeness(&release, &ctx).unwrap(),
            Completeness::Complete
        );
        let steps = publisher.publish(&release, &ctx).unwrap();
        assert_eq!(
            self::steps(&steps),
            [
                (WHEEL, &AssetStep::Uploaded),
                (SDIST, &AssetStep::AlreadyUploaded)
            ]
        );
        assert_eq!(*publisher.index.uploads.lock().unwrap(), [WHEEL]);
    }

    #[test]
    fn a_different_file_conflicts_before_any_upload() {
        let directory = tempfile::tempdir().unwrap();
        let release = release("1.2.3");
        let ctx = Context::detached();
        // The wheel sorts first and is missing; the sdist conflicts, so nothing is uploaded.
        let publisher = publisher(
            FakeIndex::with(&[(SDIST, Some("00"))]),
            distributions(directory.path()),
        );
        let PublisherState::Conflicting(conflict) = publisher.state(&release, &ctx).unwrap() else {
            panic!("expected a conflict");
        };
        assert!(
            conflict.starts_with(&format!(
                "{SDIST} on https://pypi.org/simple/ has sha256 00, expected "
            )),
            "{conflict}"
        );
        assert_eq!(
            publisher.publish(&release, &ctx).unwrap_err().to_string(),
            conflict
        );
        assert!(publisher.index.uploads.lock().unwrap().is_empty());
    }

    #[test]
    fn a_file_without_a_digest_cannot_be_verified() {
        let directory = tempfile::tempdir().unwrap();
        let release = release("1.2.3");
        let ctx = Context::detached();
        let publisher = publisher(
            FakeIndex::with(&[(SDIST, None)]),
            distributions(directory.path()),
        );
        let expected = format!(
            "{SDIST} on https://pypi.org/simple/ has no sha256 digest, so Presswerk cannot verify whether it matches"
        );
        assert_eq!(
            publisher.state(&release, &ctx).unwrap_err().to_string(),
            expected
        );
        assert_eq!(
            publisher.publish(&release, &ctx).unwrap_err().to_string(),
            expected
        );
        assert!(publisher.index.uploads.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_upload_is_accepted_only_if_the_index_holds_the_same_file() {
        let directory = tempfile::tempdir().unwrap();
        let distributions = distributions(directory.path());
        let release = release("1.2.3");
        let ctx = Context::detached();

        let failing = publisher(
            FakeIndex {
                failing_uploads: vec![WHEEL.into()],
                ..FakeIndex::default()
            },
            distributions.clone(),
        );
        let error = failing.publish(&release, &ctx).unwrap_err().to_string();
        assert_eq!(error, "HTTP 400 File already exists");

        // Another run uploaded the wheel between the state check and the upload.
        let index = FakeIndex {
            failing_uploads: vec![WHEEL.into()],
            ..FakeIndex::default()
        };
        let concurrent = publisher(index, distributions.clone());
        let wheel = distributions[0].sha256.clone();
        let publish = || {
            let ctx = Context::detached();
            concurrent.publish(&release, &ctx)
        };
        concurrent.index.files.lock().unwrap().push((
            IndexFile {
                filename: WHEEL.into(),
                sha256: Some(wheel),
            },
            1,
        ));
        assert_eq!(
            steps(&publish().unwrap()),
            [
                (WHEEL, &AssetStep::AlreadyUploaded),
                (SDIST, &AssetStep::Uploaded)
            ]
        );
    }

    #[test]
    fn waits_a_bounded_time_for_the_index_to_list_an_upload() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = publisher(
            FakeIndex {
                lag: usize::MAX,
                ..FakeIndex::default()
            },
            distributions(directory.path()),
        )
        .with_lag(Lag {
            timeout: Duration::from_millis(50),
            interval: Duration::from_millis(10),
        });
        let error = publisher
            .publish(&release("1.2.3"), &Context::detached())
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            format!(
                "https://pypi.org/simple/ does not list {WHEEL}, {SDIST} 0 seconds after the upload; publish again once it does"
            )
        );
    }

    #[test]
    fn project_and_version_are_compared_with_the_release() {
        let ctx = Context::detached();
        for (files, version, expected) in [
            (
                vec![(WHEEL, METADATA)],
                "1.2.4",
                format!("{WHEEL} has the version \"1.2.3\", expected \"1.2.4\""),
            ),
            (
                vec![("other-1.2.3.tar.gz", METADATA)],
                "1.2.3",
                "other-1.2.3.tar.gz has the project name \"other\", expected \"My.Package\"".into(),
            ),
            (
                vec![(SDIST, &METADATA.replace("Version: 1.2.3", "Version: 1.2.0"))],
                "1.2.3",
                format!("{SDIST} has the metadata version \"1.2.0\", expected \"1.2.3\""),
            ),
            (
                vec![(SDIST, &METADATA.replace("Name: My.Package", "Name: other"))],
                "1.2.3",
                format!("{SDIST} has the metadata name \"other\", expected \"My.Package\""),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            for (filename, metadata) in files {
                if filename.ends_with(".whl") {
                    wheel(directory.path(), filename, metadata);
                } else {
                    sdist(directory.path(), filename, metadata);
                }
            }
            let publisher = publisher(
                FakeIndex::default(),
                distribution::read_directory(directory.path()).unwrap(),
            );
            let release = release(version);
            assert_eq!(
                publisher.state(&release, &ctx).unwrap_err().to_string(),
                expected
            );
            assert_eq!(
                publisher.publish(&release, &ctx).unwrap_err().to_string(),
                expected
            );
            assert!(publisher.index.uploads.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn pre_releases_cannot_be_described() {
        let describe = |version: &str| {
            describe(
                "my-package",
                "https://pypi.org/simple/",
                &Version::parse(version).unwrap(),
            )
        };
        assert_eq!(
            describe("1.2.3").unwrap(),
            Description {
                target: "https://pypi.org/simple/".into(),
                items: vec![("release".into(), "my-package==1.2.3".into())],
            }
        );
        assert_eq!(
            describe("1.4.0-rc.1").unwrap_err(),
            "Python publishers cannot release the pre-release version 1.4.0-rc.1 yet"
        );
    }

    #[test]
    fn decodes_and_validates_declarations() {
        let valid = json!({
            "name": "my-package",
            "distributions": "/nix/store/00000000000000000000000000000000-dist",
            "repositoryUrl": "https://upload.pypi.org/legacy/",
            "indexUrl": "https://pypi.org/simple/",
            "username": { "value": "__token__" },
            "password": { "env": "PYPI_TOKEN" }
        });
        let python = Python::from_declaration(&valid).unwrap();
        assert_eq!(
            python.credentials(),
            [
                ("username", &Credential::Value("__token__".into())),
                ("password", &Credential::env("PYPI_TOKEN")),
            ]
        );
        assert_eq!(
            python.artifacts(),
            [(
                "distributions".into(),
                "/nix/store/00000000000000000000000000000000-dist".into()
            )]
        );
        for (option, value) in [
            ("name", json!("-my-package")),
            ("name", json!("my package")),
            ("distributions", json!("dist")),
            ("repositoryUrl", json!("ftp://upload.pypi.org/legacy/")),
            (
                "repositoryUrl",
                json!("https://user:secret@upload.pypi.org/legacy/"),
            ),
            ("indexUrl", json!("https://pypi.org/simple/?q=1")),
            ("password", json!({ "value": "secret" })),
        ] {
            let mut invalid = valid.clone();
            invalid[option] = value;
            assert!(
                matches!(
                    Python::from_declaration(&invalid),
                    Err(DeclarationError::Invalid(_))
                ),
                "{option}: {}",
                invalid[option]
            );
        }
        let mut unknown = valid;
        unknown["project"] = json!("my-package");
        assert!(matches!(
            Python::from_declaration(&unknown),
            Err(DeclarationError::IncompatibleUnstableSchema(_))
        ));
    }
}
