//! A Cargo registry: its sparse index and its publish API.

use crate::publication::PublicationError;
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;
use url::Url;

/// How long one index request may take, so a stalled index cannot hold a release.
pub const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the upload may take, archive included.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// What the index records for one version of a crate.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IndexEntry {
    pub vers: String,
    /// The hex SHA-256 of the `.crate` archive.
    pub cksum: String,
    #[serde(default)]
    pub yanked: bool,
}

/// A Cargo registry as the publisher sees it.
pub trait CargoRegistry: Send + Sync {
    /// Every version the index lists for `name`, or `None` if it lists none.
    fn versions(&self, name: &str) -> Result<Option<Vec<IndexEntry>>, PublicationError>;

    /// The URL that [`CargoRegistry::upload`] sends to, so a missing version is known to be
    /// uploadable before the Git tag is pushed.
    fn upload_url(&self) -> Result<String, PublicationError>;

    /// Sends one `PUT /api/v1/crates/new` request, returning the registry's warnings.
    fn upload(&self, body: Vec<u8>) -> Result<Vec<String>, PublicationError>;
}

/// The path of a crate's file in a sparse index, as Cargo derives it from the lowercase name.
#[must_use]
pub fn index_path(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

/// The lines of an index file. A line this CLI cannot read, such as an HTML page or a truncated
/// row, fails the read: it might be the version in question.
pub fn parse_index(file: &str) -> Result<Vec<IndexEntry>, String> {
    file.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(number, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("line {} is no index entry: {error}", number + 1))
        })
        .collect()
}

/// The `config.json` of a sparse index.
#[derive(Debug, Clone, Deserialize)]
struct IndexConfig {
    api: Option<String>,
    #[serde(default, rename = "auth-required")]
    auth_required: bool,
}

/// A registry reached over HTTP through its sparse index.
pub struct HttpRegistry {
    agent: ureq::Agent,
    /// The index URL without `sparse+`, ending in a slash.
    index: Url,
    token: Option<String>,
    config: OnceLock<IndexConfig>,
    index_timeout: Duration,
}

impl fmt::Debug for HttpRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRegistry")
            .field("index", &self.index.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpRegistry {
    /// The registry with sparse index `index`, such as `sparse+https://index.crates.io/`. Without
    /// a token it only reads, and only from an index that needs no authentication.
    pub fn new(index: &str, token: Option<String>) -> Result<Self, PublicationError> {
        let url = index.strip_prefix("sparse+").unwrap_or(index);
        let mut index = Url::parse(url).map_err(|error| {
            PublicationError::new(format!("invalid Cargo index URL {url:?}: {error}"))
        })?;
        if !index.path().ends_with('/') {
            index.set_path(&format!("{}/", index.path()));
        }
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            index,
            token,
            config: OnceLock::new(),
            index_timeout: INDEX_TIMEOUT,
        })
    }

    #[cfg(test)]
    fn with_index_timeout(mut self, timeout: Duration) -> Self {
        self.index_timeout = timeout;
        self
    }

    fn token(&self, what: &str) -> Result<&str, PublicationError> {
        self.token.as_deref().ok_or_else(|| {
            PublicationError::new(format!(
                "Cargo index {} needs the registry token to {what}",
                self.index
            ))
        })
    }

    fn get(&self, url: &Url, token: Option<&str>) -> ureq::Request {
        let request = self
            .agent
            .get(url.as_str())
            .timeout(self.index_timeout)
            .set("User-Agent", "presswerk");
        match token {
            Some(token) => request.set("Authorization", token),
            None => request,
        }
    }

    fn config(&self) -> Result<&IndexConfig, PublicationError> {
        if let Some(config) = self.config.get() {
            return Ok(config);
        }
        let url = self.join("config.json")?;
        let response = match self.get(&url, None).call() {
            // An index that needs authentication answers anonymous requests with 401; Cargo then
            // retries with the token.
            Err(ureq::Error::Status(401, _)) => self.get(&url, Some(self.token("read it")?)).call(),
            other => other,
        };
        let config = match response {
            Ok(response) => response.into_json::<IndexConfig>().map_err(|error| {
                PublicationError::new(format!("could not decode {url}: {error}"))
            })?,
            Err(error) => return Err(call_error(&format!("read {url}"), error)),
        };
        Ok(self.config.get_or_init(|| config))
    }

    fn join(&self, path: &str) -> Result<Url, PublicationError> {
        self.index.join(path).map_err(|error| {
            PublicationError::new(format!("invalid Cargo index path {path:?}: {error}"))
        })
    }
}

impl CargoRegistry for HttpRegistry {
    fn versions(&self, name: &str) -> Result<Option<Vec<IndexEntry>>, PublicationError> {
        let config = self.config()?;
        let token = if config.auth_required {
            Some(self.token("read it")?)
        } else {
            None
        };
        let url = self.join(&index_path(name))?;
        match self.get(&url, token).call() {
            Ok(response) => {
                let file = response.into_string().map_err(|error| {
                    PublicationError::new(format!("could not read {url}: {error}"))
                })?;
                parse_index(&file).map(Some).map_err(|error| {
                    PublicationError::new(format!("could not read {url}: {error}"))
                })
            }
            // Cargo reads these as "no such crate".
            Err(ureq::Error::Status(404 | 410 | 451, _)) => Ok(None),
            Err(error) => Err(call_error(&format!("read {url}"), error)),
        }
    }

    fn upload_url(&self) -> Result<String, PublicationError> {
        let api = self.config()?.api.as_deref().ok_or_else(|| {
            PublicationError::new(format!(
                "Cargo index {} names no publish API (`api` in config.json)",
                self.index
            ))
        })?;
        let invalid = || {
            PublicationError::new(format!(
                "Cargo index {} names publish API {api:?}, which is no HTTP(S) base URL",
                self.index
            ))
        };
        let mut url = Url::parse(api).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.cannot_be_a_base()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        url.path_segments_mut()
            .map_err(|()| invalid())?
            .pop_if_empty()
            .extend(["api", "v1", "crates", "new"]);
        Ok(url.into())
    }

    fn upload(&self, body: Vec<u8>) -> Result<Vec<String>, PublicationError> {
        let url = self.upload_url()?;
        let response = self
            .agent
            .put(&url)
            .timeout(UPLOAD_TIMEOUT)
            .set("User-Agent", "presswerk")
            .set("Accept", "application/json")
            .set("Content-Type", "application/octet-stream")
            .set("Authorization", self.token("publish")?)
            .send_bytes(&body)
            .map_err(|error| call_error(&format!("publish to {url}"), error))?;
        let response: Value = response.into_json().unwrap_or(Value::Null);
        // The Web API allows errors in a successful response, too.
        let details = error_details(&response);
        if !details.is_empty() {
            return Err(PublicationError::new(format!(
                "could not publish to {url}: {details}"
            )));
        }
        Ok(warnings(&response))
    }
}

/// The warnings of a successful publish response, such as unknown categories.
fn warnings(response: &Value) -> Vec<String> {
    let Some(warnings) = response.get("warnings").and_then(Value::as_object) else {
        return Vec::new();
    };
    warnings
        .iter()
        .flat_map(|(kind, values)| {
            values
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(move |value| match kind.as_str() {
                    "other" => value.to_owned(),
                    kind => format!("{}: {value}", kind.replace('_', " ")),
                })
        })
        .collect()
}

/// A failed request, with the registry's own error details where it sends them.
fn call_error(action: &str, error: ureq::Error) -> PublicationError {
    match error {
        ureq::Error::Status(status, response) => {
            let details = response
                .into_json::<Value>()
                .ok()
                .map(|body| error_details(&body))
                .filter(|details| !details.is_empty())
                .map(|details| format!(": {details}"))
                .unwrap_or_default();
            PublicationError::new(format!("could not {action} (HTTP {status}){details}"))
        }
        ureq::Error::Transport(error) => {
            PublicationError::new(format!("could not {action}: {error}"))
        }
    }
}

/// The `detail` of each error in a registry's `{ "errors": [...] }` response.
fn error_details(body: &Value) -> String {
    body.get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|error| error.get("detail").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn index_paths_follow_cargos_layout() {
        assert_eq!(index_path("a"), "1/a");
        assert_eq!(index_path("ab"), "2/ab");
        assert_eq!(index_path("abc"), "3/a/abc");
        assert_eq!(index_path("Serde_JSON"), "se/rd/serde_json");
    }

    #[test]
    fn index_files_are_read_line_by_line() {
        let file = concat!(
            r#"{"name":"demo","vers":"1.0.0","deps":[],"cksum":"aa","features":{},"yanked":false}"#,
            "\n",
            r#"{"name":"demo","vers":"1.1.0","deps":[],"cksum":"bb","features":{},"yanked":true,"v":2}"#,
            "\n\n",
        );
        assert_eq!(
            parse_index(file).unwrap(),
            [
                IndexEntry {
                    vers: "1.0.0".into(),
                    cksum: "aa".into(),
                    yanked: false
                },
                IndexEntry {
                    vers: "1.1.0".into(),
                    cksum: "bb".into(),
                    yanked: true
                },
            ]
        );
        for unreadable in [
            "<html>Service Unavailable</html>",
            r#"{"name":"demo","vers":"1.0.0","cks"#,
            r#"{"name":"demo","vers":"1.0.0","yanked":false}"#,
        ] {
            let file = format!("{}\n{unreadable}\n", file.lines().next().unwrap());
            assert!(
                parse_index(&file).unwrap_err().starts_with("line 2 "),
                "{unreadable}"
            );
        }
    }

    #[test]
    fn registry_errors_and_warnings_are_read_from_the_response() {
        assert_eq!(
            error_details(&json!({ "errors": [
                { "detail": "crate version `1.2.3` is already uploaded" },
                { "detail": "second" }
            ] })),
            "crate version `1.2.3` is already uploaded; second"
        );
        assert_eq!(
            warnings(&json!({ "warnings": {
                "invalid_categories": ["nope"],
                "invalid_badges": [],
                "other": ["be careful"]
            } })),
            ["invalid categories: nope", "be careful"]
        );
    }

    /// One recorded request: method, path, and the `Authorization` header.
    type Recorded = Arc<Mutex<Vec<(String, String, Option<String>, usize)>>>;

    type Routes = Arc<Mutex<Vec<(&'static str, String, u16, String)>>>;

    /// A registry that answers from its routes, keyed by method and path, and records every
    /// request. Routes are added once the server's address is known.
    fn serve() -> (String, Routes, Recorded) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let recorded = Recorded::default();
        let routes = Routes::default();
        let (shared, shared_routes) = (recorded.clone(), routes.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap().to_owned();
                let path = parts.next().unwrap().to_owned();
                let (mut length, mut authorization) = (0, None);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap(),
                        "authorization" => authorization = Some(value.trim().to_owned()),
                        _ => {}
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                shared
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), authorization, length));
                let (status, body) = shared_routes
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(m, p, _, _)| *m == method && *p == path)
                    .map_or((404, String::new()), |(_, _, status, body)| {
                        (*status, body.clone())
                    });
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (base, routes, recorded)
    }

    #[test]
    fn reads_the_index_and_publishes_to_the_api_it_names() {
        let (base, routes, recorded) = serve();
        routes.lock().unwrap().extend([
            (
                "GET",
                "/index/config.json".into(),
                200,
                format!(r#"{{"dl":"{base}/dl","api":"{base}/"}}"#),
            ),
            (
                "GET",
                "/index/de/mo/demo".into(),
                200,
                r#"{"name":"demo","vers":"1.0.0","cksum":"aa","yanked":false}"#.into(),
            ),
            (
                "PUT",
                "/api/v1/crates/new".into(),
                200,
                r#"{"warnings":{"other":["hi"]}}"#.into(),
            ),
        ]);
        let registry =
            HttpRegistry::new(&format!("sparse+{base}/index"), Some("secret".into())).unwrap();

        assert_eq!(registry.versions("demo").unwrap().unwrap()[0].cksum, "aa");
        assert_eq!(registry.versions("other").unwrap(), None);
        assert_eq!(registry.upload(vec![1, 2, 3]).unwrap(), ["hi"]);

        let recorded = recorded.lock().unwrap().clone();
        assert_eq!(
            recorded,
            [
                ("GET".into(), "/index/config.json".into(), None, 0),
                ("GET".into(), "/index/de/mo/demo".into(), None, 0),
                ("GET".into(), "/index/ot/he/other".into(), None, 0),
                (
                    "PUT".into(),
                    "/api/v1/crates/new".into(),
                    Some("secret".into()),
                    3
                ),
            ]
        );
    }

    #[test]
    fn an_index_that_needs_authentication_reads_with_the_token() {
        let (base, routes, recorded) = serve();
        routes.lock().unwrap().extend([
            ("GET", "/config.json".into(), 401, String::new()),
            (
                "GET",
                "/de/mo/demo".into(),
                500,
                r#"{"errors":[{"detail":"down"}]}"#.into(),
            ),
        ]);
        let registry = HttpRegistry::new(&format!("sparse+{base}"), None).unwrap();
        let error = registry.versions("demo").unwrap_err().to_string();
        assert!(error.contains("needs the registry token"), "{error}");

        // With the token, config.json is read again with it.
        let registry = HttpRegistry::new(&format!("sparse+{base}"), Some("secret".into())).unwrap();
        let error = registry.versions("demo").unwrap_err().to_string();
        assert!(error.contains("(HTTP 401)"), "{error}");
        assert_eq!(
            recorded.lock().unwrap().last().unwrap().2.as_deref(),
            Some("secret")
        );

        routes.lock().unwrap()[0] = (
            "GET",
            "/config.json".into(),
            200,
            r#"{"dl":"x","auth-required":true}"#.into(),
        );
        let registry = HttpRegistry::new(&format!("sparse+{base}"), Some("secret".into())).unwrap();
        let error = registry.versions("demo").unwrap_err().to_string();
        assert!(error.contains("(HTTP 500): down"), "{error}");
        assert_eq!(recorded.lock().unwrap().last().unwrap().1, "/de/mo/demo");
        assert_eq!(
            recorded.lock().unwrap().last().unwrap().2.as_deref(),
            Some("secret")
        );
        let error = registry.upload(Vec::new()).unwrap_err().to_string();
        assert!(error.contains("names no publish API"), "{error}");
    }

    #[test]
    fn errors_in_a_successful_upload_response_fail_the_upload() {
        let (base, routes, _) = serve();
        routes.lock().unwrap().extend([
            (
                "GET",
                "/config.json".into(),
                200,
                format!(r#"{{"dl":"x","api":"{base}"}}"#),
            ),
            (
                "PUT",
                "/api/v1/crates/new".into(),
                200,
                r#"{"errors":[{"detail":"crate name is reserved"}]}"#.into(),
            ),
        ]);
        let registry =
            HttpRegistry::new(&format!("sparse+{base}/"), Some("secret".into())).unwrap();
        let error = registry.upload(vec![1]).unwrap_err().to_string();
        assert!(error.ends_with(": crate name is reserved"), "{error}");
    }

    #[test]
    fn an_api_that_is_no_http_url_is_not_uploadable() {
        let (base, routes, _) = serve();
        routes.lock().unwrap().push((
            "GET",
            "/config.json".into(),
            200,
            r#"{"dl":"x","api":"ftp://registry.example"}"#.into(),
        ));
        let registry =
            HttpRegistry::new(&format!("sparse+{base}/"), Some("secret".into())).unwrap();
        let error = registry.upload_url().unwrap_err().to_string();
        assert!(error.contains("which is no HTTP(S) base URL"), "{error}");

        // A query or fragment would swallow the route.
        for api in [
            "https://registry.example/?x=1",
            "https://registry.example/#top",
            "https://user:pw@registry.example/",
        ] {
            routes.lock().unwrap()[0].3 = format!(r#"{{"dl":"x","api":"{api}"}}"#);
            let registry =
                HttpRegistry::new(&format!("sparse+{base}/"), Some("secret".into())).unwrap();
            assert!(registry.upload_url().is_err(), "{api}");
        }
        for (api, url) in [
            ("https://crates.io", "https://crates.io/api/v1/crates/new"),
            (
                "https://kellnr.example/prefix/",
                "https://kellnr.example/prefix/api/v1/crates/new",
            ),
        ] {
            routes.lock().unwrap()[0].3 = format!(r#"{{"dl":"x","api":"{api}"}}"#);
            let registry =
                HttpRegistry::new(&format!("sparse+{base}/"), Some("secret".into())).unwrap();
            assert_eq!(registry.upload_url().unwrap(), url);
        }
    }

    #[test]
    fn a_stalled_index_times_out() {
        // Accepts the connection, then never answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(2).collect();
            thread::sleep(std::time::Duration::from_secs(10));
            drop(held);
        });
        let registry = HttpRegistry::new(&format!("sparse+{base}/"), None)
            .unwrap()
            .with_index_timeout(std::time::Duration::from_millis(100));
        let start = std::time::Instant::now();
        assert!(registry.versions("demo").is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
}
