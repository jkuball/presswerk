//! OCI registries: the client and the formats shared by the OCI image and Helm publishers.

pub mod helm;
pub mod image;

use crate::publication::{ArtifactKind, PublicationError};
use crate::publishers::Labels;
use crate::publishers::shared::secrets::{Credential, SecretSource, Secrets};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::sync::Mutex;
use url::Url;

pub const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
pub const GZIP_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub const ZSTD_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+zstd";
/// The index annotation that records the release version, so aliases can be compared.
pub const VERSION_ANNOTATION: &str = "org.opencontainers.image.version";
/// The annotation on each manifest in an index that records the Nix store path of its image, so a
/// later run recognizes the release's index without comparing compressed bytes.
pub const ORIGIN_ANNOTATION: &str = "dev.presswerk.origin";

const VERSION_PLACEHOLDER: &str = "{version}";
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";
/// A tag that names the release version, of an image or a chart.
pub const TAG: ArtifactKind = ArtifactKind::new("tag", "pushed");

const LABELS: Labels = Labels {
    publisher: "registry",
    artifacts: "tags",
};

/// Where a registry publisher pushes, and with which credentials.
pub(crate) struct Registry<'a> {
    pub repository: &'a str,
    pub public: bool,
    pub username: &'a Credential,
    pub password: &'a Credential,
}

impl Registry<'_> {
    fn validate(&self) -> Result<(), String> {
        RepositoryReference::parse(self.repository)?;
        self.username
            .validate()
            .and_then(|()| self.password.validate_secret("registry password"))
            .map_err(|error| error.to_string())
    }

    fn credentials(&self, secrets: &Secrets) -> Result<Credentials, PublicationError> {
        Credentials::resolve(self.username, self.password, secrets)
    }

    /// A client that pushes with the credentials.
    fn writing_client(&self, secrets: &Secrets) -> Result<RegistryClient, PublicationError> {
        RegistryClient::new(self.repository, Some(self.credentials(secrets)?))
    }

    /// A client that reads as `plan` and `prepare` do: anonymously if the repository is public, so
    /// its credentials are not needed.
    fn reading_client(&self, secrets: &Secrets) -> Result<RegistryClient, PublicationError> {
        let credentials = if self.public {
            None
        } else {
            Some(self.credentials(secrets)?)
        };
        RegistryClient::new(self.repository, credentials)
    }

    /// The credential variables that are unset or empty, as `lookup` reads them; none for a
    /// public repository, which `plan` and `prepare` read anonymously.
    fn missing_credentials(&self, lookup: impl Fn(&str) -> Option<String>) -> Vec<String> {
        if self.public {
            return Vec::new();
        }
        [self.username, self.password]
            .into_iter()
            .filter_map(|credential| match credential {
                Credential::Secret(SecretSource::Env(name)) => Some(name),
                Credential::Value(_) => None,
            })
            .filter(|name| lookup(name).is_none_or(|value| value.is_empty()))
            .cloned()
            .collect()
    }
}

/// A registry repository such as `ghcr.io/owner/project`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryReference {
    /// The registry's base URL, such as `https://ghcr.io/`.
    pub registry: Url,
    /// The repository name within the registry, such as `owner/project`.
    pub name: String,
}

impl RepositoryReference {
    /// Parse `host[:port]/path`. The host is required, so an image never lands on a default
    /// registry by accident. Loopback hosts use plain HTTP, like Docker's default insecure
    /// registries; every other registry uses HTTPS.
    pub fn parse(repository: &str) -> Result<Self, String> {
        let invalid = |reason: &str| format!("OCI repository {repository:?} {reason}");
        let (host, name) = repository
            .split_once('/')
            .ok_or_else(|| invalid("must be a registry host followed by a repository path"))?;
        if !(host.contains('.') || host.contains(':') || host == "localhost") {
            return Err(invalid(
                "must start with a registry host, such as ghcr.io/owner/project",
            ));
        }
        // Only a host and port: credentials belong in environment variables, never in the
        // declaration, and a query or fragment has no meaning for a registry.
        if !valid_host(host) {
            return Err(invalid(
                "must start with a plain registry host and optional port, without credentials, query, or fragment",
            ));
        }
        if name.is_empty()
            || !name.split('/').all(|component| {
                !component.is_empty()
                    && component
                        .bytes()
                        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
                    && component.starts_with(|c: char| c.is_ascii_alphanumeric())
                    && component.ends_with(|c: char| c.is_ascii_alphanumeric())
            })
        {
            return Err(invalid(
                "must have a lowercase repository path of letters, digits, and separators",
            ));
        }
        let hostname = host.rsplit_once(':').map_or(host, |(hostname, port)| {
            if port.bytes().all(|byte| byte.is_ascii_digit()) {
                hostname
            } else {
                host
            }
        });
        let scheme = if matches!(hostname, "localhost" | "127.0.0.1" | "[::1]") {
            "http"
        } else {
            "https"
        };
        // Docker Hub answers under several hosts and names official images without `library/`;
        // its registry API is registry-1.docker.io.
        let (host, name) = if matches!(
            host,
            "docker.io" | "index.docker.io" | "registry-1.docker.io"
        ) {
            let name = if name.contains('/') {
                name.to_owned()
            } else {
                format!("library/{name}")
            };
            ("registry-1.docker.io", name)
        } else {
            (host, name.to_owned())
        };
        let registry = Url::parse(&format!("{scheme}://{host}/"))
            .map_err(|error| invalid(&format!("has an invalid registry host: {error}")))?;
        if registry.path() != "/" || registry.query().is_some() {
            return Err(invalid("has an invalid registry host"));
        }
        Ok(Self { registry, name })
    }
}

/// A registry host: a DNS name, IPv4 address, or bracketed IPv6 address, with an optional port.
fn valid_host(host: &str) -> bool {
    let (name_valid, port) = if let Some(rest) = host.strip_prefix('[') {
        let Some((address, port)) = rest.split_once(']') else {
            return false;
        };
        (
            !address.is_empty()
                && address
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b':'),
            port,
        )
    } else {
        let (name, port) = host
            .find(':')
            .map_or((host, ""), |index| (&host[..index], &host[index..]));
        (
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
            port,
        )
    };
    name_valid
        && (port.is_empty()
            || port.strip_prefix(':').is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            }))
}

/// The platform an image runs on, such as `linux/amd64` or `linux/arm/v7`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl Platform {
    pub fn parse(platform: &str) -> Result<Self, String> {
        let parts: Vec<_> = platform.split('/').collect();
        let valid = |part: &&str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        };
        match parts.as_slice() {
            [os, architecture] | [os, architecture, _]
                if parts.iter().all(valid) && !os.is_empty() && !architecture.is_empty() =>
            {
                Ok(Self {
                    os: (*os).into(),
                    architecture: (*architecture).into(),
                    variant: parts.get(2).map(|variant| (*variant).into()),
                })
            }
            _ => Err(format!(
                "image platform {platform:?} must have the form os/architecture[/variant], such as linux/amd64"
            )),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.os, self.architecture)?;
        if let Some(variant) = &self.variant {
            write!(formatter, "/{variant}")?;
        }
        Ok(())
    }
}

pub(crate) use crate::publishers::shared::digest::{
    hex_digest, sha256_digest, sha256_digest_reader,
};

/// A blob of an image, as it is uploaded: a compressed layer or the config.
#[derive(Debug)]
pub struct Blob<'a> {
    pub digest: String,
    pub size: u64,
    pub data: BlobData<'a>,
}

#[derive(Debug)]
pub enum BlobData<'a> {
    /// A compressed layer too large to keep in memory, in an unnamed temporary file that is gone
    /// once the blob is handled.
    File(&'a File),
    Bytes(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

impl Descriptor {
    pub(crate) fn new(media_type: &str, digest: &str, size: u64) -> Self {
        Self {
            media_type: media_type.into(),
            digest: digest.into(),
            size,
            platform: None,
            annotations: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub(crate) schema_version: u32,
    pub(crate) media_type: String,
    pub(crate) config: Descriptor,
    pub(crate) layers: Vec<Descriptor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) annotations: BTreeMap<String, String>,
}

/// Registry credentials, read from the environment at run time.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credentials([redacted])")
    }
}

impl Credentials {
    /// The credentials, with their secrets taken from `secrets`.
    pub fn resolve(
        username: &Credential,
        password: &Credential,
        secrets: &Secrets,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            username: secrets.get(username)?,
            password: secrets.get(password)?,
        })
    }
}

enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    File(&'a File, u64),
}

enum CallError {
    Status(u16),
    Transport(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(formatter, "HTTP {status}"),
            Self::Transport(error) => formatter.write_str(error),
        }
    }
}

/// A client for one repository of an OCI distribution registry.
pub struct RegistryClient {
    agent: ureq::Agent,
    repository: RepositoryReference,
    display: String,
    /// `None` reads anonymously, for repositories anyone may pull from.
    credentials: Option<Credentials>,
    authorization: Mutex<Option<String>>,
}

impl fmt::Debug for RegistryClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryClient")
            .field("repository", &self.display)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

impl RegistryClient {
    /// A client for `repository`; without credentials it only pulls, anonymously.
    pub fn new(
        repository: &str,
        credentials: Option<Credentials>,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            repository: RepositoryReference::parse(repository).map_err(PublicationError::new)?,
            display: repository.into(),
            credentials,
            authorization: Mutex::new(None),
        })
    }

    fn url(&self, kind: &str, reference: &str) -> Url {
        self.repository
            .registry
            .join(&format!("v2/{}/{kind}/{reference}", self.repository.name))
            .expect("registry URLs are valid")
    }

    fn call(
        &self,
        method: &str,
        url: &Url,
        headers: &[(&str, &str)],
        body: &Body<'_>,
    ) -> Result<ureq::Response, CallError> {
        let mut authenticated = false;
        loop {
            let mut request = self
                .agent
                .request(method, url.as_str())
                .set("User-Agent", "presswerk");
            for (name, value) in headers {
                request = request.set(name, value);
            }
            if let Some(authorization) = self.authorization.lock().expect("lock").as_deref() {
                request = request.set("Authorization", authorization);
            }
            let result = match body {
                Body::Empty => request.call(),
                Body::Bytes(bytes) => request.send_bytes(bytes),
                Body::File(file, size) => {
                    // A retry after authenticating sends the file again from its start.
                    let mut file = *file;
                    file.rewind().map_err(|error| {
                        CallError::Transport(format!("could not read a compressed layer: {error}"))
                    })?;
                    request
                        .set("Content-Length", &size.to_string())
                        .send(BufReader::new(file))
                }
            };
            match result {
                Ok(response) => return Ok(response),
                Err(ureq::Error::Status(401, response)) if !authenticated => {
                    authenticated = true;
                    let challenge = response.header("WWW-Authenticate").unwrap_or_default();
                    let authorization = self.authenticate(challenge)?;
                    *self.authorization.lock().expect("lock") = Some(authorization);
                }
                Err(ureq::Error::Status(status @ (401 | 403), _)) if self.credentials.is_none() => {
                    return Err(self.not_public(&format!("the registry answered HTTP {status}")));
                }
                Err(ureq::Error::Status(status, _)) => return Err(CallError::Status(status)),
                Err(ureq::Error::Transport(error)) => {
                    return Err(CallError::Transport(error.to_string()));
                }
            }
        }
    }

    fn not_public(&self, reason: &str) -> CallError {
        CallError::Transport(format!(
            "{reason} to an anonymous read, so {} does not seem to be publicly readable; is `public = true` correct?",
            self.display
        ))
    }

    /// Answer a registry's authentication challenge with the configured credentials, or
    /// anonymously with pull access only.
    fn authenticate(&self, challenge: &str) -> Result<String, CallError> {
        let basic = self.credentials.as_ref().map(|credentials| {
            format!(
                "Basic {}",
                base64(format!("{}:{}", credentials.username, credentials.password))
            )
        });
        let (scheme, parameters) = challenge.split_once(' ').unwrap_or((challenge, ""));
        if scheme.eq_ignore_ascii_case("basic") {
            return basic.ok_or_else(|| self.not_public("the registry asked for a password"));
        }
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(CallError::Transport(format!(
                "registry asked for unsupported authentication {scheme:?}"
            )));
        }
        let parameters = challenge_parameters(parameters);
        let realm = parameters.get("realm").ok_or_else(|| {
            CallError::Transport("registry sent a bearer challenge without a realm".into())
        })?;
        let mut url = Url::parse(realm).map_err(|error| {
            CallError::Transport(format!("registry sent an invalid token realm: {error}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = parameters.get("service") {
                query.append_pair("service", service);
            }
            let actions = if basic.is_some() { "pull,push" } else { "pull" };
            query.append_pair(
                "scope",
                &format!("repository:{}:{actions}", self.repository.name),
            );
        }
        let mut request = self.agent.get(url.as_str()).set("User-Agent", "presswerk");
        if let Some(basic) = &basic {
            request = request.set("Authorization", basic);
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) if basic.is_none() => {
                return Err(self.not_public(&format!(
                    "the registry token service answered HTTP {status}"
                )));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(CallError::Transport(format!(
                    "registry token service rejected the credentials (HTTP {status})"
                )));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(CallError::Transport(format!(
                    "could not reach the registry token service: {error}"
                )));
            }
        };
        #[derive(Deserialize)]
        struct TokenResponse {
            token: Option<String>,
            access_token: Option<String>,
        }
        let token: TokenResponse = response.into_json().map_err(|error| {
            CallError::Transport(format!("invalid registry token response: {error}"))
        })?;
        token
            .token
            .or(token.access_token)
            .filter(|token| !token.is_empty())
            .map(|token| format!("Bearer {token}"))
            .ok_or_else(|| CallError::Transport("registry token response has no token".into()))
    }

    fn unknown(&self, what: &str, error: &CallError) -> PublicationError {
        PublicationError::new(format!(
            "could not inspect {what} in {}: {error}; Presswerk cannot tell whether it exists",
            self.display
        ))
    }

    /// The digest a tag or digest reference resolves to, or `None` if the registry has none.
    pub fn manifest_digest(&self, reference: &str) -> Result<Option<String>, PublicationError> {
        let url = self.url("manifests", reference);
        match self.call("HEAD", &url, &[("Accept", MANIFEST_ACCEPT)], &Body::Empty) {
            Ok(response) => match response.header("Docker-Content-Digest") {
                Some(digest) if digest.starts_with("sha256:") => Ok(Some(digest.to_owned())),
                _ => Ok(self.manifest(reference)?.map(|(digest, _)| digest)),
            },
            Err(CallError::Status(404)) => Ok(None),
            Err(error) => Err(self.unknown(&format!("manifest {reference}"), &error)),
        }
    }

    /// The manifest a reference resolves to, with the digest of its bytes.
    pub fn manifest(&self, reference: &str) -> Result<Option<(String, Vec<u8>)>, PublicationError> {
        let url = self.url("manifests", reference);
        match self.call("GET", &url, &[("Accept", MANIFEST_ACCEPT)], &Body::Empty) {
            Ok(response) => {
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .take(16 * 1024 * 1024)
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        self.unknown(
                            &format!("manifest {reference}"),
                            &CallError::Transport(error.to_string()),
                        )
                    })?;
                Ok(Some((sha256_digest(&bytes), bytes)))
            }
            Err(CallError::Status(404)) => Ok(None),
            Err(error) => Err(self.unknown(&format!("manifest {reference}"), &error)),
        }
    }

    pub fn blob_exists(&self, digest: &str) -> Result<bool, PublicationError> {
        match self.call("HEAD", &self.url("blobs", digest), &[], &Body::Empty) {
            Ok(_) => Ok(true),
            Err(CallError::Status(404)) => Ok(false),
            Err(error) => Err(self.unknown(&format!("blob {digest}"), &error)),
        }
    }

    /// Upload a blob unless the registry already has it. Returns whether it was uploaded.
    pub fn push_blob(&self, blob: &Blob<'_>) -> Result<bool, PublicationError> {
        if self.blob_exists(&blob.digest)? {
            return Ok(false);
        }
        self.upload_blob(blob)?;
        Ok(true)
    }

    /// Upload a blob in one request; a blob that appears meanwhile counts as uploaded.
    pub fn upload_blob(&self, blob: &Blob<'_>) -> Result<(), PublicationError> {
        let failure = match self.start_upload() {
            Ok(mut location) => {
                location
                    .query_pairs_mut()
                    .append_pair("digest", &blob.digest);
                match self.call(
                    "PUT",
                    &location,
                    &[("Content-Type", "application/octet-stream")],
                    &match blob.data {
                        BlobData::File(path) => Body::File(path, blob.size),
                        BlobData::Bytes(bytes) => Body::Bytes(bytes),
                    },
                ) {
                    Ok(_) => return Ok(()),
                    Err(error) => error.to_string(),
                }
            }
            Err(error) => error,
        };
        if self.blob_exists(&blob.digest)? {
            return Ok(());
        }
        Err(PublicationError::new(format!(
            "could not upload blob {} to {}: {failure}",
            blob.digest, self.display
        )))
    }

    fn start_upload(&self) -> Result<Url, String> {
        let url = self
            .repository
            .registry
            .join(&format!("v2/{}/blobs/uploads/", self.repository.name))
            .expect("registry URLs are valid");
        let response = self
            .call("POST", &url, &[], &Body::Bytes(&[]))
            .map_err(|error| error.to_string())?;
        let location = response
            .header("Location")
            .ok_or("the registry started an upload without a Location")?;
        url.join(location)
            .map_err(|error| format!("the registry sent an invalid upload location: {error}"))
    }

    /// Put a manifest under its digest unless the registry already has it.
    pub fn push_manifest(
        &self,
        media_type: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<bool, PublicationError> {
        if self.manifest_digest(digest)?.is_some() {
            return Ok(false);
        }
        self.put_manifest(digest, media_type, bytes, digest)?;
        Ok(true)
    }

    /// Put a manifest under a tag or digest; a failed write is inspected again, and succeeds
    /// only if the reference now holds exactly this manifest.
    pub fn put_manifest(
        &self,
        reference: &str,
        media_type: &str,
        bytes: &[u8],
        digest: &str,
    ) -> Result<(), PublicationError> {
        let url = self.url("manifests", reference);
        let failure = match self.call(
            "PUT",
            &url,
            &[("Content-Type", media_type)],
            &Body::Bytes(bytes),
        ) {
            Ok(response) => match response.header("Docker-Content-Digest") {
                Some(actual) if actual != digest => {
                    return Err(PublicationError::new(format!(
                        "{} stored {reference} with digest {actual}, expected {digest}",
                        self.display
                    )));
                }
                _ => return Ok(()),
            },
            Err(error) => error.to_string(),
        };
        if self.manifest_digest(reference)?.as_deref() == Some(digest) {
            return Ok(());
        }
        Err(PublicationError::new(format!(
            "could not push {}:{reference}: {failure}",
            self.display
        )))
    }
}

fn challenge_parameters(parameters: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut rest = parameters.trim();
    while let Some((key, value)) = rest.split_once('=') {
        let key = key
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let value = value.trim_start();
        let (value, remainder) = if let Some(quoted) = value.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            (&quoted[..end], quoted.get(end + 1..).unwrap_or(""))
        } else {
            let end = value.find(',').unwrap_or(value.len());
            (&value[..end], &value[end..])
        };
        result.insert(key, value.to_owned());
        rest = remainder.trim_start_matches([',', ' ']);
    }
    result
}

fn base64(input: impl AsRef<[u8]>) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.as_ref();
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                output.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                output.push('=');
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_repositories() {
        let ghcr = RepositoryReference::parse("ghcr.io/owner/project").unwrap();
        assert_eq!(ghcr.registry.as_str(), "https://ghcr.io/");
        assert_eq!(ghcr.name, "owner/project");

        let local = RepositoryReference::parse("localhost:5000/team/image").unwrap();
        assert_eq!(local.registry.as_str(), "http://localhost:5000/");
        let loopback = RepositoryReference::parse("127.0.0.1:5000/image").unwrap();
        assert_eq!(loopback.registry.as_str(), "http://127.0.0.1:5000/");

        let hub = RepositoryReference::parse("docker.io/nginx").unwrap();
        assert_eq!(hub.registry.as_str(), "https://registry-1.docker.io/");
        assert_eq!(hub.name, "library/nginx");

        let ipv6 = RepositoryReference::parse("[::1]:5000/image").unwrap();
        assert_eq!(ipv6.registry.as_str(), "http://[::1]:5000/");

        let gar = RepositoryReference::parse("europe-docker.pkg.dev/project/repo/image").unwrap();
        assert_eq!(gar.name, "project/repo/image");

        for invalid in [
            "owner/project",
            "ghcr.io",
            "ghcr.io/",
            "ghcr.io/Owner/project",
            "ghcr.io/owner//project",
            "ghcr.io/owner/project:tag",
            "ghcr.io/-owner/project",
            "user:secret@ghcr.io/owner/project",
            "token@ghcr.io/owner/project",
            "ghcr.io?x=1/owner/project",
            "ghcr.io#fragment/owner/project",
            "ghcr.io:port/owner/project",
            "ghcr.io/owner/project?x=1",
            "ghcr.io/owner/project#fragment",
        ] {
            assert!(RepositoryReference::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn every_docker_hub_spelling_pushes_to_its_registry() {
        for spelling in [
            "docker.io/alpine",
            "index.docker.io/alpine",
            "registry-1.docker.io/alpine",
            "registry-1.docker.io/library/alpine",
        ] {
            let parsed = RepositoryReference::parse(spelling).unwrap();
            assert_eq!(
                (parsed.registry.as_str(), parsed.name.as_str()),
                ("https://registry-1.docker.io/", "library/alpine"),
                "{spelling}"
            );
        }
    }

    #[test]
    fn parses_platforms() {
        assert_eq!(
            Platform::parse("linux/arm/v7").unwrap().to_string(),
            "linux/arm/v7"
        );
        assert_eq!(Platform::parse("linux/amd64").unwrap().variant, None);
        for invalid in ["linux", "linux/", "Linux/amd64", "linux/amd64/v8/x"] {
            assert!(Platform::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_authentication_challenges() {
        let parameters = challenge_parameters(
            r#"realm="https://ghcr.io/token",service="ghcr.io",scope="repository:a/b:pull,push""#,
        );
        assert_eq!(parameters["realm"], "https://ghcr.io/token");
        assert_eq!(parameters["service"], "ghcr.io");
        assert_eq!(parameters["scope"], "repository:a/b:pull,push");
    }

    #[test]
    fn encodes_basic_credentials() {
        assert_eq!(base64("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64("a"), "YQ==");
        assert_eq!(base64("ab"), "YWI=");
        assert_eq!(base64(""), "");
    }

    #[test]
    fn credentials_never_appear_in_debug_output() {
        let credentials = Credentials {
            username: "user".into(),
            password: "secret-token".into(),
        };
        assert!(!format!("{credentials:?}").contains("secret-token"));
    }
}
