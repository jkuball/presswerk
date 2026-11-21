//! A Cargo registry: its sparse index and its publish API.

use crate::publication::PublicationError;
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::sync::OnceLock;
use url::Url;

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

/// The lines of an index file, skipping those this CLI cannot read, such as newer formats.
#[must_use]
pub fn parse_index(file: &str) -> Vec<IndexEntry> {
    file.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
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
        })
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
        let request = self.agent.get(url.as_str()).set("User-Agent", "presswerk");
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
                Ok(Some(parse_index(&file)))
            }
            // Cargo reads these as "no such crate".
            Err(ureq::Error::Status(404 | 410 | 451, _)) => Ok(None),
            Err(error) => Err(call_error(&format!("read {url}"), error)),
        }
    }

    fn upload(&self, body: Vec<u8>) -> Result<Vec<String>, PublicationError> {
        let api = self.config()?.api.clone().ok_or_else(|| {
            PublicationError::new(format!(
                "Cargo index {} names no publish API (`api` in config.json)",
                self.index
            ))
        })?;
        let url = format!("{}/api/v1/crates/new", api.trim_end_matches('/'));
        let response = self
            .agent
            .put(&url)
            .set("User-Agent", "presswerk")
            .set("Accept", "application/json")
            .set("Content-Type", "application/octet-stream")
            .set("Authorization", self.token("publish")?)
            .send_bytes(&body)
            .map_err(|error| call_error(&format!("publish to {url}"), error))?;
        let response: Value = response.into_json().unwrap_or(Value::Null);
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
            "\n\nnot json\n",
        );
        assert_eq!(
            parse_index(file),
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
}
