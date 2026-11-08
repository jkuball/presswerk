mod support;

use presswerk::analysis::{Analysis, Bump};
use presswerk::declaration::ReleaseDeclaration;
use presswerk::publication::{
    GitTagPublisher, GithubClient, GithubPublisher, GithubReleaseReader, GithubReleaseState,
    PublicationError, Publisher, PublisherState, PublisherStep, Release, RemoteState, TagPublisher,
    TagStep, is_valid_successor, publish_ordered, validate_new_release, verify_predecessor_with,
};
use presswerk::release_commit::TrustedPredecessor;
use semver::Version;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::rc::Rc;
use std::thread;
use support::GitRepo;

fn declaration(version: &str, repository: &str) -> ReleaseDeclaration {
    ReleaseDeclaration::from_json(&format!(
        r#"{{
          "schemaVersion": 2,
          "version": "{version}",
          "versionFiles": [],
          "tag": {{ "format": "v{{version}}" }},
          "analyzer": {{
            "convention": "gitmoji",
            "gitmoji": {{ "catalog": {{}} }}
          }},
          "changelog": {{ "path": "CHANGELOG.md" }},
          "publishers": {{
            "github-release": {{
              "default": {{
                "apiUrl": "https://api.github.com",
                "repository": "{repository}",
                "token": {{ "secret": "GITHUB_TOKEN" }}
              }}
            }}
          }}
        }}"#
    ))
    .unwrap()
}

fn release(commit: String) -> Release {
    Release {
        commit,
        version: Version::new(0, 1, 0),
        tag: "v0.1.0".into(),
        notes: "Frozen notes.\n".into(),
    }
}

#[derive(Clone)]
struct FakeTagPublisher {
    state: Rc<RefCell<RemoteState>>,
    writes: Rc<RefCell<Vec<&'static str>>>,
}

impl TagPublisher for FakeTagPublisher {
    fn state(&self, _release: &Release) -> Result<RemoteState, PublicationError> {
        Ok(self.state.borrow().clone())
    }

    fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
        self.writes.borrow_mut().push("tag");
        *self.state.borrow_mut() = RemoteState::Matching;
        Ok(())
    }
}

#[derive(Clone)]
struct FakePublisher {
    state: Rc<RefCell<PublisherState>>,
    writes: Rc<RefCell<Vec<&'static str>>>,
}

impl Publisher for FakePublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(self.state.borrow().clone())
    }

    fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
        self.writes.borrow_mut().push("release");
        *self.state.borrow_mut() = PublisherState::Published;
        Ok(())
    }
}

#[test]
fn publishes_the_tag_before_completing_missing_publishers() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Rc::new(RefCell::new(PublisherState::Draft)),
        writes: writes.clone(),
    };

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[Box::new(publisher)],
    )
    .unwrap();

    assert_eq!(*writes.borrow(), ["tag", "release"]);
    assert_eq!(
        steps,
        (TagStep::Pushed, vec![PublisherStep::DraftCompleted])
    );
}

#[test]
fn reports_complete_publication_without_writing_again() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Rc::new(RefCell::new(PublisherState::Published)),
        writes: writes.clone(),
    };

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[Box::new(publisher)],
    )
    .unwrap();

    assert_eq!(
        steps,
        (
            TagStep::AlreadyPushed,
            vec![PublisherStep::AlreadyPublished]
        )
    );
    assert_eq!(*writes.borrow(), ["tag"]);
}

#[test]
fn refuses_an_empty_publisher_set_before_writing_the_tag() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Missing)),
        writes: writes.clone(),
    };

    let error = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[],
    )
    .unwrap_err();

    assert!(error.to_string().contains("at least one"));
    assert!(writes.borrow().is_empty());
}

#[test]
fn preflight_conflicts_stop_before_the_tag_is_written() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Rc::new(RefCell::new(PublisherState::Conflicting(
            "wrong prerelease flag".into(),
        ))),
        writes: writes.clone(),
    };

    let error = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[Box::new(publisher)],
    )
    .unwrap_err();

    assert!(error.to_string().contains("wrong prerelease flag"));
    assert!(writes.borrow().is_empty());
}

#[test]
fn validates_successors_changelog_and_the_analyzed_minimum() {
    let parent = Version::new(1, 2, 3);
    let candidate = Version::parse("1.3.0-rc.1").unwrap();
    let analysis = Analysis {
        bump: Some(Bump::Minor),
        commits: Vec::new(),
    };
    let changelog = "# Changelog\n\n## [1.3.0-rc.1] - 2026-11-05\n\nCandidate notes.\n";

    validate_new_release(&candidate, &parent, Some(&parent), &analysis, changelog).unwrap();

    let too_small = Version::new(1, 2, 4);
    assert!(
        validate_new_release(
            &too_small,
            &parent,
            Some(&parent),
            &analysis,
            "## [1.2.4] - 2026-11-05\n\nFix.\n",
        )
        .unwrap_err()
        .to_string()
        .contains("analyzed minimum")
    );
    assert!(
        validate_new_release(
            &Version::new(1, 4, 0),
            &parent,
            Some(&parent),
            &analysis,
            "## [1.4.0] - 2026-11-05\n\nJump.\n",
        )
        .unwrap_err()
        .to_string()
        .contains("valid successor")
    );
    assert!(
        validate_new_release(
            &candidate,
            &parent,
            Some(&parent),
            &analysis,
            "# Changelog\n",
        )
        .unwrap_err()
        .to_string()
        .contains("does not contain")
    );
}

#[test]
fn accepts_semver_prerelease_steps_and_stable_successors() {
    assert!(is_valid_successor(
        &Version::parse("1.3.0-rc.1").unwrap(),
        &Version::parse("1.3.0-rc.2").unwrap()
    ));
    assert!(is_valid_successor(
        &Version::parse("1.3.0-rc.2").unwrap(),
        &Version::new(1, 3, 0)
    ));
    assert!(is_valid_successor(
        &Version::new(0, 4, 2),
        &Version::new(1, 0, 0)
    ));
    assert!(!is_valid_successor(
        &Version::new(1, 2, 3),
        &Version::new(1, 4, 0)
    ));
}

#[test]
fn pushes_an_annotated_tag_without_force_and_reuses_it() {
    let repository = GitRepo::new();
    let commit = repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    let publisher = GitTagPublisher::new(repository.path(), "origin");
    let release = release(commit.clone());

    assert_eq!(publisher.state(&release).unwrap(), RemoteState::Missing);
    publisher.publish(&release).unwrap();
    assert_eq!(publisher.state(&release).unwrap(), RemoteState::Matching);
    publisher.publish(&release).unwrap();

    let tagged = Command::new("git")
        .args([
            "--git-dir",
            &remote_path,
            "rev-parse",
            "refs/tags/v0.1.0^{commit}",
        ])
        .output()
        .unwrap();
    assert!(tagged.status.success());
    assert_eq!(String::from_utf8(tagged.stdout).unwrap().trim(), commit);
    let tag_type = Command::new("git")
        .args([
            "--git-dir",
            &remote_path,
            "cat-file",
            "-t",
            "refs/tags/v0.1.0",
        ])
        .output()
        .unwrap();
    assert!(tag_type.status.success());
    assert_eq!(String::from_utf8(tag_type.stdout).unwrap().trim(), "tag");
}

#[test]
fn refuses_to_push_a_new_lightweight_tag() {
    let repository = GitRepo::new();
    let commit = repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    repository.tag("v0.1.0");
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    let publisher = GitTagPublisher::new(repository.path(), "origin");

    let error = publisher.publish(&release(commit)).unwrap_err();

    assert!(error.to_string().contains("not annotated"));
    assert_eq!(
        publisher.state(&release(repository.head())).unwrap(),
        RemoteState::Missing
    );
}

#[test]
fn treats_an_existing_remote_lightweight_tag_as_conflicting() {
    let repository = GitRepo::new();
    let commit = repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    repository.tag("v0.1.0");
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    repository.git(["push", "origin", "refs/tags/v0.1.0"]);
    let publisher = GitTagPublisher::new(repository.path(), "origin");

    let state = publisher.state(&release(commit)).unwrap();

    assert!(matches!(state, RemoteState::Conflicting(message) if message.contains("lightweight")));
}

#[test]
fn refuses_to_overwrite_a_conflicting_remote_tag() {
    let repository = GitRepo::new();
    let first = repository.commit_version("0.1.0", "🔖 First release");
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    let publisher = GitTagPublisher::new(repository.path(), "origin");
    publisher.publish(&release(first.clone())).unwrap();

    repository.write("fix", "next\n");
    let second = repository.commit("🐛 Fix after release");
    let conflicting = release(second);

    assert!(matches!(
        publisher.state(&conflicting).unwrap(),
        RemoteState::Conflicting(_)
    ));
    assert!(publisher.publish(&conflicting).is_err());
    assert_eq!(
        publisher.state(&release(first)).unwrap(),
        RemoteState::Matching
    );
}

#[test]
fn predecessor_verification_uses_the_tagged_commits_own_publishers() {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("publisher", "new/project\n");
    let candidate = repository.commit("✨ Change publisher");
    let current_declaration = declaration("0.1.0", "new/project");
    let previous_declaration = declaration("0.1.0", "old/project");
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: Rc::new(RefCell::new(Vec::new())),
    };
    let repositories = RefCell::new(Vec::new());

    let verified = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        None,
        |commit| {
            assert_eq!(commit.to_string(), predecessor);
            Ok(previous_declaration.clone())
        },
        |publisher, _tag, _version| {
            repositories.borrow_mut().push(publisher.repository.clone());
            Ok(GithubReleaseState::Published { prerelease: false })
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(verified.commit.to_string(), predecessor);
    assert_eq!(*repositories.borrow(), ["old/project"]);
}

#[test]
fn unsupported_predecessor_requires_the_exact_trust_and_a_matching_remote_tag() {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let predecessor = predecessor.parse().unwrap();
    let candidate = candidate.parse().unwrap();
    let current_declaration = declaration("0.1.0", "new/project");
    let unsupported = PublicationError::unsupported_predecessor("v0.1.0", predecessor, 1);

    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: Rc::new(RefCell::new(Vec::new())),
    };
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        None,
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
    )
    .unwrap_err();
    assert_eq!(
        error.as_unsupported_predecessor().unwrap().commit,
        predecessor
    );

    let forged = TrustedPredecessor {
        tag: "v0.1.0".into(),
        commit: candidate,
    };
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&forged),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not match expected"));

    let trusted = TrustedPredecessor {
        tag: "v0.1.0".into(),
        commit: predecessor,
    };
    *tag.state.borrow_mut() = RemoteState::Missing;
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing from the remote"));

    *tag.state.borrow_mut() = RemoteState::Matching;
    let verified = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(verified.commit, predecessor);
}

#[test]
fn trusted_predecessor_must_be_an_ancestor() {
    let repository = GitRepo::new();
    let root = repository.commit_version("0.1.0", "🎉 Begin project");
    repository.git(["checkout", "--quiet", "-b", "released"]);
    repository.write("release", "prepared\n");
    let predecessor = repository.commit("🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.git(["checkout", "--quiet", "-b", "candidate", &root]);
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let predecessor = predecessor.parse().unwrap();
    let trusted = TrustedPredecessor {
        tag: "v0.1.0".into(),
        commit: predecessor,
    };
    let current_declaration = declaration("0.1.0", "new/project");
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: Rc::new(RefCell::new(Vec::new())),
    };

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| unreachable!("ancestry is checked before decoding"),
        |_, _, _| unreachable!("ancestry is checked before publishers"),
    )
    .unwrap_err();

    assert!(error.to_string().contains("not an ancestor"));
}

#[test]
fn supported_predecessors_reject_unnecessary_trust() {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let predecessor = predecessor.parse().unwrap();
    let current_declaration = declaration("0.1.0", "new/project");
    let previous_declaration = declaration("0.1.0", "old/project");
    let trusted = TrustedPredecessor {
        tag: "v0.1.0".into(),
        commit: predecessor,
    };
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: Rc::new(RefCell::new(Vec::new())),
    };

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Ok(previous_declaration.clone()),
        |_, _, _| Ok(GithubReleaseState::Published { prerelease: false }),
    )
    .unwrap_err();

    assert!(error.to_string().contains("unnecessary"));
}

#[test]
fn predecessor_verification_rejects_an_empty_tagged_publisher_set() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Prepare invalid predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("publisher", "new/project\n");
    let candidate = repository.commit("✨ Change publisher");
    let current_declaration = declaration("0.1.0", "new/project");
    let mut previous_declaration = declaration("0.1.0", "old/project");
    previous_declaration.publishers.github_release.clear();
    let tag = FakeTagPublisher {
        state: Rc::new(RefCell::new(RemoteState::Matching)),
        writes: Rc::new(RefCell::new(Vec::new())),
    };

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        None,
        |_commit| Ok(previous_declaration.clone()),
        |_publisher, _tag, _version| Ok(GithubReleaseState::Published { prerelease: false }),
    )
    .unwrap_err();

    assert!(error.to_string().contains("at least one"));
}

#[derive(Clone)]
struct ExpectedRequest {
    method: &'static str,
    path: &'static str,
    status: u16,
    response: &'static str,
    body_contains: Option<&'static str>,
}

fn mock_github_api(requests: Vec<ExpectedRequest>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for expected in requests {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let mut lines = request.lines();
            let first = lines.next().unwrap();
            assert_eq!(
                first,
                format!("{} {} HTTP/1.1", expected.method, expected.path)
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token")
            );
            if let Some(body) = expected.body_contains {
                assert!(request.contains(body), "request was {request:?}");
            }
            let reason = match expected.status {
                200 => "OK",
                201 => "Created",
                404 => "Not Found",
                500 => "Internal Server Error",
                status => panic!("unsupported test status {status}"),
            };
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                expected.status,
                reason,
                expected.response.len(),
                expected.response
            )
            .unwrap();
        }
    });
    (format!("http://{address}"), handle)
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let mut body_end = None;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
        if body_end.is_none()
            && let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            body_end = Some(position + 4);
        }
        if let Some(body_start) = body_end {
            let headers = String::from_utf8_lossy(&bytes[..body_start]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|length| length.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= body_start + content_length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

#[test]
fn github_missing_is_distinct_from_inaccessible_or_unknown_state() {
    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v1.2.3",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: "[]",
            body_contains: None,
        },
    ]);
    let client = GithubClient::new(&url, "test-token").unwrap();
    assert_eq!(
        client.release_state("owner/project", "v1.2.3").unwrap(),
        GithubReleaseState::Missing
    );
    server.join().unwrap();

    let (url, server) = mock_github_api(vec![ExpectedRequest {
        method: "GET",
        path: "/repos/owner/project",
        status: 404,
        response: "{}",
        body_contains: None,
    }]);
    let client = GithubClient::new(&url, "test-token").unwrap();
    assert!(
        client
            .release_state("owner/project", "v1.2.3")
            .unwrap_err()
            .to_string()
            .contains("inaccessible")
    );
    server.join().unwrap();

    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v1.2.3",
            status: 500,
            response: "{}",
            body_contains: None,
        },
    ]);
    let client = GithubClient::new(&url, "test-token").unwrap();
    assert!(client.release_state("owner/project", "v1.2.3").is_err());
    server.join().unwrap();
}

#[test]
fn github_publishes_a_draft_found_only_in_the_release_listing() {
    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: Some(r#""draft":false"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: None,
        },
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    );

    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();

    server.join().unwrap();
}

fn tag_lookup_and_listing(pages: Vec<String>) -> Vec<ExpectedRequest> {
    let mut requests = vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
    ];
    for (index, page) in pages.into_iter().enumerate() {
        requests.push(ExpectedRequest {
            method: "GET",
            path: format!(
                "/repos/owner/project/releases?per_page=100&page={}",
                index + 1
            )
            .leak(),
            status: 200,
            response: page.leak(),
            body_contains: None,
        });
    }
    requests
}

fn published_releases(count: usize) -> String {
    let releases: Vec<_> = (0..count)
        .map(|index| {
            format!(
                r#"{{"id":{},"tag_name":"v0.0.{index}","draft":false,"prerelease":false}}"#,
                100 + index
            )
        })
        .collect();
    format!("[{}]", releases.join(","))
}

#[test]
fn github_finds_a_draft_on_a_later_listing_page_by_exact_tag() {
    let (url, server) = mock_github_api(tag_lookup_and_listing(vec![
        published_releases(100),
        r#"[{"id":8,"tag_name":"v0.1.0-rc.1","draft":true,"prerelease":true},{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#.into(),
    ]));
    let client = GithubClient::new(&url, "test-token").unwrap();

    assert_eq!(
        client.release_state("owner/project", "v0.1.0").unwrap(),
        GithubReleaseState::Draft { prerelease: false }
    );
    server.join().unwrap();
}

#[test]
fn github_missing_release_reads_every_listing_page() {
    let (url, server) = mock_github_api(tag_lookup_and_listing(vec![
        published_releases(100),
        "[]".into(),
    ]));
    let client = GithubClient::new(&url, "test-token").unwrap();

    assert_eq!(
        client.release_state("owner/project", "v0.1.0").unwrap(),
        GithubReleaseState::Missing
    );
    server.join().unwrap();
}

#[test]
fn github_rejects_listed_releases_without_a_tag_name() {
    for (entry, expected) in [
        (
            r#"[{"id":7,"draft":true,"prerelease":false}]"#,
            "could not decode GitHub Releases",
        ),
        (
            r#"[{"id":7,"tag_name":"","draft":true,"prerelease":false}]"#,
            "without a tag name",
        ),
    ] {
        // The mock serves only the lookup and the listing, so publishing must stop before a POST.
        let (url, server) = mock_github_api(tag_lookup_and_listing(vec![entry.into()]));
        let publisher = GithubPublisher::new(
            GithubClient::new(&url, "test-token").unwrap(),
            "owner/project",
        );

        let error = publisher
            .publish(&release("0123456789012345678901234567890123456789".into()))
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{entry}: {error}");
        server.join().unwrap();
    }
}

#[test]
fn github_rejects_several_drafts_for_one_tag() {
    let (url, server) = mock_github_api(tag_lookup_and_listing(vec![
        r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false},{"id":8,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#.into(),
    ]));
    let client = GithubClient::new(&url, "test-token").unwrap();

    let error = client.release_state("owner/project", "v0.1.0").unwrap_err();
    assert!(
        error.to_string().contains("found 2 GitHub Releases"),
        "{error}"
    );
    server.join().unwrap();
}

#[test]
fn github_reinspects_ambiguous_post_and_patch_failures() {
    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: "[]",
            body_contains: None,
        },
        ExpectedRequest {
            method: "POST",
            path: "/repos/owner/project/releases",
            status: 500,
            response: "{}",
            body_contains: Some(r#""draft":true"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: Some(r#""draft":false"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: None,
        },
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    );
    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();
    server.join().unwrap();

    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 500,
            response: "{}",
            body_contains: Some(r#""draft":false"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: None,
        },
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    );
    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();
    server.join().unwrap();
}

#[test]
fn github_reinspection_rejects_a_still_draft_release() {
    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 500,
            response: "{}",
            body_contains: Some(r#""draft":false"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false}]"#,
            body_contains: None,
        },
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    );

    let error = publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap_err();

    assert!(error.to_string().contains("could not publish draft"));
    server.join().unwrap();
}

#[test]
fn github_creates_a_draft_then_publishes_and_verifies_it() {
    let (url, server) = mock_github_api(vec![
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 404,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases?per_page=100&page=1",
            status: 200,
            response: "[]",
            body_contains: None,
        },
        ExpectedRequest {
            method: "POST",
            path: "/repos/owner/project/releases",
            status: 201,
            response: r#"{"id":7,"draft":true,"prerelease":false}"#,
            body_contains: Some(r#""draft":true"#),
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: Some(r#""draft":false"#),
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project",
            status: 200,
            response: "{}",
            body_contains: None,
        },
        ExpectedRequest {
            method: "GET",
            path: "/repos/owner/project/releases/tags/v0.1.0",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false}"#,
            body_contains: None,
        },
    ]);
    let client = GithubClient::new(&url, "test-token").unwrap();
    let publisher = GithubPublisher::new(client, "owner/project");

    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();

    server.join().unwrap();
}
