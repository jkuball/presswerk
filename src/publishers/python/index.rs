//! A package index: the simple API ([PEP 691] JSON, or [PEP 503] HTML) that state is read from,
//! and the legacy upload API that distributions are uploaded through.
//!
//! [PEP 691]: https://peps.python.org/pep-0691/
//! [PEP 503]: https://peps.python.org/pep-0503/

use super::distribution::{Distribution, normalize};
use crate::publication::PublicationError;
use crate::publishers::shared;
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

/// One file the simple API lists for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFile {
    pub filename: String,
    /// Its SHA-256 digest as lowercase hex, if the index reports one.
    pub sha256: Option<String>,
}

/// What the publisher needs of an index; tests use a fake.
pub trait Index: Send + Sync {
    /// Every file the simple API lists for `project`, none if it does not know the project.
    fn files(&self, project: &str) -> Result<Vec<IndexFile>, PublicationError>;

    /// Uploads one distribution through the legacy upload API.
    fn upload(&self, distribution: &Distribution) -> Result<(), PublicationError>;
}

/// Basic authentication for an index.
#[derive(Clone)]
pub struct Login {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Login {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Login")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

/// An index over HTTP.
#[derive(Debug, Clone)]
pub struct HttpIndex {
    agent: ureq::Agent,
    repository_url: Url,
    index_url: Url,
    /// Uploads need it; reads send it only when the index asks for it, as pip does, so a public
    /// index never receives it for a read.
    login: Option<Login>,
}

const SIMPLE_ACCEPT: &str = "application/vnd.pypi.simple.v1+json, application/vnd.pypi.simple.v1+html;q=0.2, text/html;q=0.01";

impl HttpIndex {
    pub fn new(
        repository_url: &str,
        index_url: &str,
        login: Option<Login>,
    ) -> Result<Self, PublicationError> {
        let parse = |url: &str| {
            Url::parse(url)
                .map_err(|error| PublicationError::new(format!("invalid URL {url:?}: {error}")))
        };
        let mut index_url = parse(index_url)?;
        if !index_url.path().ends_with('/') {
            index_url.set_path(&format!("{}/", index_url.path()));
        }
        Ok(Self {
            agent: ureq::AgentBuilder::new().build(),
            repository_url: parse(repository_url)?,
            index_url,
            login,
        })
    }

    fn project_url(&self, project: &str) -> Result<Url, PublicationError> {
        self.index_url
            .join(&format!("{}/", normalize(project)))
            .map_err(|error| PublicationError::new(format!("invalid project URL: {error}")))
    }

    fn get(&self, url: &Url, login: Option<&Login>) -> ureq::Request {
        let request = self
            .agent
            .get(url.as_str())
            .set("Accept", SIMPLE_ACCEPT)
            // A CDN may still serve the page from before an upload.
            .set("Cache-Control", "no-cache")
            .set("User-Agent", "presswerk");
        match login {
            Some(login) => request.set("Authorization", &basic(login)),
            None => request,
        }
    }
}

impl Index for HttpIndex {
    fn files(&self, project: &str) -> Result<Vec<IndexFile>, PublicationError> {
        let url = self.project_url(project)?;
        let mut result = self.get(&url, None).call();
        if let (Err(ureq::Error::Status(401 | 403, _)), Some(login)) = (&result, &self.login) {
            result = self.get(&url, Some(login)).call();
        }
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(404, _)) => return Ok(Vec::new()),
            Err(ureq::Error::Status(status, _)) => {
                return Err(PublicationError::new(format!(
                    "could not read {url} (HTTP {status})"
                )));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(PublicationError::new(format!(
                    "could not read {url}: {error}"
                )));
            }
        };
        let content_type = response.content_type().to_owned();
        let body = response
            .into_string()
            .map_err(|error| PublicationError::new(format!("could not read {url}: {error}")))?;
        parse_simple(&content_type, &body)
            .map_err(|error| PublicationError::new(format!("could not decode {url}: {error}")))
    }

    fn upload(&self, distribution: &Distribution) -> Result<(), PublicationError> {
        let login = self.login.as_ref().ok_or_else(|| {
            PublicationError::new("uploads need a username and password".to_owned())
        })?;
        let mut content = Vec::new();
        std::fs::File::open(&distribution.path)
            .and_then(|mut file| file.read_to_end(&mut content))
            .map_err(|error| {
                PublicationError::new(format!(
                    "could not read {}: {error}",
                    distribution.path.display()
                ))
            })?;
        let (content_type, body) = upload_form(distribution, &content);
        let failure = match self
            .agent
            .post(self.repository_url.as_str())
            .set("Authorization", &basic(login))
            .set("Content-Type", &content_type)
            .set("User-Agent", "presswerk")
            .send_bytes(&body)
        {
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(status, response)) => {
                // PyPI explains a rejection in the status text, others in the body.
                let reason = response.status_text().to_owned();
                let body = response.into_string().unwrap_or_default();
                let detail = body.lines().map(str::trim).find(|line| !line.is_empty());
                match detail {
                    Some(detail) if detail != reason => {
                        format!("HTTP {status} {reason}: {}", truncate(detail))
                    }
                    _ => format!("HTTP {status} {reason}"),
                }
            }
            Err(ureq::Error::Transport(error)) => error.to_string(),
        };
        Err(PublicationError::new(format!(
            "could not upload {} to {}: {failure}",
            distribution.filename, self.repository_url
        )))
    }
}

fn truncate(text: &str) -> String {
    const LIMIT: usize = 300;
    match text.char_indices().nth(LIMIT) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

fn basic(login: &Login) -> String {
    format!(
        "Basic {}",
        shared::base64(format!("{}:{}", login.username, login.password))
    )
}

/// The files of a simple API project page, as JSON ([PEP 691]) or HTML ([PEP 503]) by its
/// content type.
///
/// [PEP 691]: https://peps.python.org/pep-0691/
/// [PEP 503]: https://peps.python.org/pep-0503/
pub fn parse_simple(content_type: &str, body: &str) -> Result<Vec<IndexFile>, String> {
    if content_type.contains("json") {
        #[derive(Deserialize)]
        struct Page {
            files: Vec<File>,
        }
        #[derive(Deserialize)]
        struct File {
            filename: String,
            #[serde(default)]
            hashes: BTreeMap<String, String>,
        }
        let page: Page = serde_json::from_str(body).map_err(|error| error.to_string())?;
        return Ok(page
            .files
            .into_iter()
            .map(|file| IndexFile {
                filename: file.filename,
                sha256: file
                    .hashes
                    .get("sha256")
                    .map(|digest| digest.to_ascii_lowercase()),
            })
            .collect());
    }
    static ANCHOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<a\s[^>]*?href\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*>(.*?)</a\s*>"#)
            .expect("valid anchor pattern")
    });
    Ok(ANCHOR
        .captures_iter(body)
        .map(|anchor| {
            let href = anchor
                .get(1)
                .or_else(|| anchor.get(2))
                .map_or("", |m| m.as_str());
            let sha256 = href.split_once('#').and_then(|(_, fragment)| {
                fragment
                    .strip_prefix("sha256=")
                    .map(str::to_ascii_lowercase)
            });
            IndexFile {
                filename: unescape(anchor[3].trim()),
                sha256,
            }
        })
        .collect())
}

/// The few HTML entities a filename can contain.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// The `multipart/form-data` body of an upload through the legacy upload API, with its content
/// type: the distribution's core metadata, what the filename says, its digest, and the file.
#[must_use]
pub fn upload_form(distribution: &Distribution, content: &[u8]) -> (String, Vec<u8>) {
    // A file never contains a boundary derived from its own digest.
    let boundary = format!("presswerk-{}", distribution.sha256);
    let mut body = Vec::new();
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                escape(name)
            )
            .as_bytes(),
        );
    };
    field(":action", "file_upload");
    field("protocol_version", "1");
    for (name, value) in &distribution.metadata {
        field(name, value);
    }
    field("filetype", distribution.parsed.filetype.form_value());
    field("pyversion", &distribution.parsed.pyversion);
    field("sha256_digest", &distribution.sha256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"content\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
            escape(&distribution.filename)
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publishers::python::distribution::{Filename, form_fields};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    #[test]
    fn reads_the_json_simple_api() {
        // Recorded from https://pypi.org/simple/presswerk-example/, shortened.
        let body = r#"{"meta":{"api-version":"1.4","_last-serial":1},"name":"my-package","versions":["1.2.3"],"files":[
            {"filename":"my_package-1.2.3-py3-none-any.whl","url":"https://files.example/a.whl","hashes":{"sha256":"AB12"},"requires-python":">=3.9","core-metadata":{"sha256":"ff"},"size":1,"upload-time":"2026-01-01T00:00:00.000000Z","yanked":false},
            {"filename":"my_package-1.2.3.tar.gz","url":"https://files.example/a.tar.gz","hashes":{},"size":1}
        ]}"#;
        assert_eq!(
            parse_simple("application/vnd.pypi.simple.v1+json", body).unwrap(),
            [
                IndexFile {
                    filename: "my_package-1.2.3-py3-none-any.whl".into(),
                    sha256: Some("ab12".into()),
                },
                IndexFile {
                    filename: "my_package-1.2.3.tar.gz".into(),
                    sha256: None,
                },
            ]
        );
    }

    #[test]
    fn reads_the_html_simple_api() {
        // As Google Artifact Registry and other PEP 503 indexes serve it.
        let body = r#"<!DOCTYPE html>
<html><head><meta name="pypi:repository-version" content="1.0"><title>Links for my-package</title></head>
<body><h1>Links for my-package</h1>
<a href="https://files.example/my_package-1.2.3-py3-none-any.whl#sha256=CD34" data-requires-python="&gt;=3.9">my_package-1.2.3-py3-none-any.whl</a><br/>
<A HREF='../../files/my_package-1.2.3.tar.gz#md5=00'>
  my_package-1.2.3.tar.gz
</A><br/>
</body></html>"#;
        assert_eq!(
            parse_simple("text/html; charset=utf-8", body).unwrap(),
            [
                IndexFile {
                    filename: "my_package-1.2.3-py3-none-any.whl".into(),
                    sha256: Some("cd34".into()),
                },
                IndexFile {
                    filename: "my_package-1.2.3.tar.gz".into(),
                    sha256: None,
                },
            ]
        );
    }

    #[test]
    fn builds_the_upload_form() {
        let distribution = Distribution {
            filename: "my_package-1.2.3-py3-none-any.whl".into(),
            path: PathBuf::from("/nix/store/x-dist/my_package-1.2.3-py3-none-any.whl"),
            parsed: Filename::parse("my_package-1.2.3-py3-none-any.whl").unwrap(),
            sha256: "ab12".into(),
            metadata: form_fields("Metadata-Version: 2.4\nName: my-package\nVersion: 1.2.3\n"),
        };
        let (content_type, body) = upload_form(&distribution, b"WHEEL");
        assert_eq!(content_type, "multipart/form-data; boundary=presswerk-ab12");
        let field = |name: &str, value: &str| {
            format!(
                "--presswerk-ab12\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
        };
        let expected = [
            field(":action", "file_upload"),
            field("protocol_version", "1"),
            field("metadata_version", "2.4"),
            field("name", "my-package"),
            field("version", "1.2.3"),
            field("filetype", "bdist_wheel"),
            field("pyversion", "py3"),
            field("sha256_digest", "ab12"),
            "--presswerk-ab12\r\nContent-Disposition: form-data; name=\"content\"; filename=\"my_package-1.2.3-py3-none-any.whl\"\r\nContent-Type: application/octet-stream\r\n\r\nWHEEL\r\n--presswerk-ab12--\r\n".into(),
        ]
        .concat();
        assert_eq!(String::from_utf8(body).unwrap(), expected);
    }

    #[test]
    fn the_index_url_is_a_directory() {
        let index = HttpIndex::new(
            "https://upload.pypi.org/legacy/",
            "https://pypi.org/simple",
            None,
        )
        .unwrap();
        assert_eq!(
            index.project_url("My.Package").unwrap().as_str(),
            "https://pypi.org/simple/my-package/"
        );
    }

    /// Serves each request with the next response and records each request's line and
    /// `Authorization` header.
    fn serve(responses: Vec<(u16, &'static str, &'static str)>) -> (String, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || {
            for (status, content_type, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut request = request.trim_end().to_owned();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    let lower = header.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if lower.starts_with("authorization:") {
                        request.push_str(&format!(" [{}]", header.trim_end()));
                    }
                }
                let mut body_read = vec![0; length];
                reader.read_exact(&mut body_read).unwrap();
                sender.send(request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {status} Status\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (address, requests)
    }

    fn login() -> Option<Login> {
        Some(Login {
            username: "user".into(),
            password: "presswerk-test-index-password".into(),
        })
    }

    #[test]
    fn reads_anonymously_until_the_index_asks_for_credentials() {
        let json = "application/vnd.pypi.simple.v1+json";
        let (address, requests) = serve(vec![
            (200, json, r#"{"files":[]}"#),
            (401, "text/plain", "Unauthorized"),
            (
                200,
                json,
                r#"{"files":[{"filename":"a-1.0.0.tar.gz","hashes":{"sha256":"ab"}}]}"#,
            ),
            (404, "text/plain", "Not Found"),
        ]);
        let index = HttpIndex::new(
            &format!("{address}/legacy/"),
            &format!("{address}/simple"),
            login(),
        )
        .unwrap();
        assert_eq!(index.files("My.Package").unwrap(), []);
        assert_eq!(
            index.files("My.Package").unwrap(),
            [IndexFile {
                filename: "a-1.0.0.tar.gz".into(),
                sha256: Some("ab".into()),
            }]
        );
        // An index that does not know the project lists no files.
        assert_eq!(index.files("My.Package").unwrap(), []);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(
            requests,
            [
                "GET /simple/my-package/ HTTP/1.1",
                "GET /simple/my-package/ HTTP/1.1",
                "GET /simple/my-package/ HTTP/1.1 [Authorization: Basic dXNlcjpwcmVzc3dlcmstdGVzdC1pbmRleC1wYXNzd29yZA==]",
                "GET /simple/my-package/ HTTP/1.1",
            ]
        );
    }

    #[test]
    fn uploads_with_credentials_and_reports_the_rejection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a-1.0.0.tar.gz");
        std::fs::write(&path, "SDIST").unwrap();
        let distribution = Distribution {
            filename: "a-1.0.0.tar.gz".into(),
            path,
            parsed: Filename::parse("a-1.0.0.tar.gz").unwrap(),
            sha256: "ab".into(),
            metadata: Vec::new(),
        };
        let (address, requests) = serve(vec![
            (200, "text/plain", ""),
            (400, "text/html", "\n  File already exists.\n"),
        ]);
        let index = HttpIndex::new(
            &format!("{address}/legacy/"),
            &format!("{address}/simple/"),
            login(),
        )
        .unwrap();
        index.upload(&distribution).unwrap();
        let error = index.upload(&distribution).unwrap_err().to_string();
        assert_eq!(
            error,
            format!(
                "could not upload a-1.0.0.tar.gz to {address}/legacy/: HTTP 400 Status: File already exists."
            )
        );
        assert_eq!(
            requests.try_iter().next().unwrap(),
            "POST /legacy/ HTTP/1.1 [Authorization: Basic dXNlcjpwcmVzc3dlcmstdGVzdC1pbmRleC1wYXNzd29yZA==]"
        );

        let anonymous = HttpIndex::new(
            &format!("{address}/legacy/"),
            &format!("{address}/simple/"),
            None,
        )
        .unwrap();
        assert_eq!(
            anonymous.upload(&distribution).unwrap_err().to_string(),
            "uploads need a username and password"
        );
    }
}
