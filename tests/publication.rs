mod support;

use git2::Oid;
use presswerk::analysis::{Analysis, Bump};
use presswerk::changelog::ChangelogHeading;
use presswerk::declaration::{
    Compression, OciHelmPublisher, OciImagePublisher, ReleaseDeclaration, Token, Username,
};
use presswerk::publication::{
    ArtifactKind, ArtifactStep, ArtifactSteps, AssetStep, GITHUB_RELEASE_BODY_LIMIT,
    GitTagPublisher, GithubClient, GithubPublisher, GithubReleaseReader, GithubReleaseState,
    LocalAsset, PublicationError, PublishEvent, Publisher, PublisherNode, PublisherOutcome,
    PublisherState, PublisherStep, Release, RemoteState, TagPublisher, TagStep,
    UnreadableDeclaration, UntaggedRelease, is_valid_successor, pending_predecessor,
    publish_ordered, publish_ordered_with_jobs, validate_new_release, verify_predecessor_with,
    verify_predecessor_with_oci_helm,
};
use presswerk::release_commit::TrustedPredecessor;
use semver::Version;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use support::GitRepo;

fn declaration(version: &str, repository: &str) -> ReleaseDeclaration {
    let stable = Version::parse(version).unwrap().major >= 1;
    ReleaseDeclaration::from_json(&format!(
        r#"{{
          "schemaVersion": 0,
          "version": "{version}",
          "stable": {stable},
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
    state: Arc<Mutex<RemoteState>>,
    writes: Arc<Mutex<Vec<&'static str>>>,
}

impl TagPublisher for FakeTagPublisher {
    fn state(&self, _release: &Release) -> Result<RemoteState, PublicationError> {
        Ok(self.state.lock().unwrap().clone())
    }

    fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
        self.writes.lock().unwrap().push("tag");
        *self.state.lock().unwrap() = RemoteState::Matching;
        Ok(())
    }
}

fn fake_tag(state: RemoteState) -> FakeTagPublisher {
    FakeTagPublisher {
        state: Arc::new(Mutex::new(state)),
        writes: Arc::new(Mutex::new(Vec::new())),
    }
}

#[derive(Clone)]
struct FakePublisher {
    state: Arc<Mutex<PublisherState>>,
    writes: Arc<Mutex<Vec<&'static str>>>,
}

impl Publisher for FakePublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(self.state.lock().unwrap().clone())
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        _progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        self.writes.lock().unwrap().push("release");
        *self.state.lock().unwrap() = PublisherState::Published;
        Ok(Vec::new())
    }
}

#[test]
fn publishes_the_tag_before_completing_missing_publishers() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Arc::new(Mutex::new(PublisherState::Draft)),
        writes: writes.clone(),
    };

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[PublisherNode::new("test", Box::new(publisher), Vec::new())],
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(*writes.lock().unwrap(), ["tag", "release"]);
    assert_eq!(
        steps,
        (
            TagStep::Pushed,
            vec![PublisherOutcome::Published {
                step: PublisherStep::DraftCompleted,
                artifacts: Vec::new(),
            }]
        )
    );
}

#[test]
fn reports_each_step_as_soon_as_it_completes() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: log.clone(),
    };

    struct Streaming {
        log: Arc<Mutex<Vec<&'static str>>>,
        published: AtomicBool,
    }
    impl Publisher for Streaming {
        fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
            Ok(if self.published.load(Ordering::SeqCst) {
                PublisherState::Published
            } else {
                PublisherState::Missing
            })
        }
        fn publish_with_progress(
            &self,
            _release: &Release,
            progress: &mut dyn FnMut(&ArtifactStep),
        ) -> Result<ArtifactSteps, PublicationError> {
            let steps: ArtifactSteps = ["a", "b"]
                .into_iter()
                .map(|name| ArtifactStep {
                    kind: ArtifactKind::Asset,
                    name: name.into(),
                    step: AssetStep::Uploaded,
                })
                .collect();
            for step in &steps {
                self.log.lock().unwrap().push("upload");
                progress(step);
            }
            self.published.store(true, Ordering::SeqCst);
            Ok(steps)
        }
    }

    let publishers = [PublisherNode::new(
        "github-release.default",
        Box::new(Streaming {
            log: log.clone(),
            published: AtomicBool::new(false),
        }),
        Vec::new(),
    )];
    publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        false,
        &mut |event| {
            log.lock().unwrap().push(match event {
                PublishEvent::Started { .. } => "started",
                PublishEvent::Tag { .. } => "tag reported",
                PublishEvent::Artifact(_) => "upload reported",
                PublishEvent::Publisher { .. } => "publisher reported",
                PublishEvent::PublisherFailed { .. } => "publisher failed",
                PublishEvent::PublisherSkipped { .. } => "publisher skipped",
            });
        },
    )
    .unwrap();

    assert_eq!(
        *log.lock().unwrap(),
        [
            "tag",
            "tag reported",
            "upload",
            "upload reported",
            "upload",
            "upload reported",
            "publisher reported",
        ]
    );
}

#[test]
fn pending_aliases_do_not_keep_a_release_from_completing() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Matching)),
        writes: writes.clone(),
    };
    // A publisher whose aliases stay pending, such as after a failed `latest` update.
    let publisher = FakePublisher {
        state: Arc::new(Mutex::new(PublisherState::AliasesPending)),
        writes: writes.clone(),
    };

    struct Stuck(FakePublisher);
    impl Publisher for Stuck {
        fn state(&self, release: &Release) -> Result<PublisherState, PublicationError> {
            self.0.state(release)
        }
        fn publish_with_progress(
            &self,
            _release: &Release,
            _progress: &mut dyn FnMut(&ArtifactStep),
        ) -> Result<ArtifactSteps, PublicationError> {
            self.0.writes.lock().unwrap().push("aliases");
            Ok(Vec::new())
        }
    }

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[PublisherNode::new(
            "test",
            Box::new(Stuck(publisher)),
            Vec::new(),
        )],
        false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(writes.lock().unwrap().last(), Some(&"aliases"));
    assert_eq!(
        steps.1,
        vec![PublisherOutcome::Published {
            step: PublisherStep::ArtifactsCompleted,
            artifacts: Vec::new(),
        }]
    );
}

#[test]
fn reports_complete_publication_without_writing_again() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Matching)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Arc::new(Mutex::new(PublisherState::Published)),
        writes: writes.clone(),
    };

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[PublisherNode::new("test", Box::new(publisher), Vec::new())],
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(
        steps,
        (
            TagStep::AlreadyPushed,
            vec![PublisherOutcome::Published {
                step: PublisherStep::AlreadyPublished,
                artifacts: Vec::new(),
            }]
        )
    );
    assert_eq!(*writes.lock().unwrap(), ["tag"]);
}

#[test]
fn a_release_without_publishers_only_pushes_its_tag() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[],
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(steps, (TagStep::Pushed, Vec::new()));
    assert_eq!(*writes.lock().unwrap(), ["tag"]);
}

#[test]
fn preflight_conflicts_stop_before_the_tag_is_written() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publisher = FakePublisher {
        state: Arc::new(Mutex::new(PublisherState::Conflicting(
            "wrong prerelease flag".into(),
        ))),
        writes: writes.clone(),
    };

    let error = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[PublisherNode::new("test", Box::new(publisher), Vec::new())],
        false,
        &mut |_| {},
    )
    .unwrap_err();

    assert!(error.to_string().contains("wrong prerelease flag"));
    assert!(writes.lock().unwrap().is_empty());
}

struct OrderedPublisher {
    name: &'static str,
    writes: Arc<Mutex<Vec<&'static str>>>,
    published: AtomicBool,
    fail: bool,
}

impl Publisher for OrderedPublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(if self.published.load(Ordering::SeqCst) {
            PublisherState::Published
        } else {
            PublisherState::Missing
        })
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        _progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        self.writes.lock().unwrap().push(self.name);
        if self.fail {
            return Err(PublicationError::new(format!("{} broke", self.name)));
        }
        self.published.store(true, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

fn ordered_node(
    name: &'static str,
    after: &[&str],
    writes: Arc<Mutex<Vec<&'static str>>>,
    fail: bool,
) -> PublisherNode {
    PublisherNode::new(
        name,
        Box::new(OrderedPublisher {
            name,
            writes,
            published: AtomicBool::new(false),
            fail,
        }),
        after.iter().map(|name| (*name).to_owned()).collect(),
    )
}

#[test]
fn publishes_ready_nodes_serially_in_stable_topological_order() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publishers = [
        ordered_node("b", &["a"], writes.clone(), false),
        ordered_node("a", &[], writes.clone(), false),
        ordered_node("c", &[], writes.clone(), false),
    ];

    let (_, outcomes) = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(*writes.lock().unwrap(), ["tag", "a", "b", "c"]);
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome,
        PublisherOutcome::Published {
            step: PublisherStep::Published,
            ..
        }
    )));
}

#[test]
fn a_failed_publisher_skips_descendants_but_not_independent_publishers() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publishers = [
        ordered_node("worker", &[], writes.clone(), true),
        ordered_node("release", &["worker"], writes.clone(), false),
        ordered_node("docs", &[], writes.clone(), false),
    ];
    let events = Arc::new(Mutex::new(Vec::new()));
    let reported = events.clone();

    let (_, outcomes) = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        false,
        &mut |event| match event {
            PublishEvent::PublisherFailed { name, reason }
            | PublishEvent::PublisherSkipped { name, reason } => {
                reported.lock().unwrap().push(format!("{name}: {reason}"));
            }
            _ => {}
        },
    )
    .unwrap();

    assert_eq!(*writes.lock().unwrap(), ["tag", "worker", "docs"]);
    assert_eq!(
        outcomes,
        [
            PublisherOutcome::Failed {
                reason: "worker broke".into(),
            },
            PublisherOutcome::Skipped {
                reason: "skipped: worker failed".into(),
            },
            PublisherOutcome::Published {
                step: PublisherStep::Published,
                artifacts: Vec::new(),
            },
        ]
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["worker: worker broke", "release: skipped: worker failed"]
    );
}

#[test]
fn a_failed_publisher_skips_transitive_descendants_before_it_in_node_order() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publishers = [
        ordered_node("app", &["lib"], writes.clone(), false),
        ordered_node("lib", &["zbase"], writes.clone(), false),
        ordered_node("zbase", &[], writes.clone(), true),
    ];

    let (_, outcomes) = publish_ordered_with_jobs(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        NonZeroUsize::new(2).unwrap(),
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(*writes.lock().unwrap(), ["tag", "zbase"]);
    assert_eq!(
        outcomes,
        [
            PublisherOutcome::Skipped {
                reason: "skipped: zbase failed".into(),
            },
            PublisherOutcome::Skipped {
                reason: "skipped: zbase failed".into(),
            },
            PublisherOutcome::Failed {
                reason: "zbase broke".into(),
            },
        ]
    );
}

#[test]
fn fail_fast_skips_every_publisher_after_the_first_failure() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publishers = [
        ordered_node("worker", &[], writes.clone(), true),
        ordered_node("docs", &[], writes.clone(), false),
    ];

    let (_, outcomes) = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        true,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(*writes.lock().unwrap(), ["tag", "worker"]);
    assert_eq!(
        outcomes[1],
        PublisherOutcome::Skipped {
            reason: "skipped: fail-fast after worker failed".into(),
        }
    );
}

#[test]
fn preflight_checks_every_publisher_before_rejecting_a_conflict() {
    struct Checked {
        calls: Arc<AtomicUsize>,
        state: PublisherState,
    }
    impl Publisher for Checked {
        fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.state.clone())
        }
        fn publish_with_progress(
            &self,
            _release: &Release,
            _progress: &mut dyn FnMut(&ArtifactStep),
        ) -> Result<ArtifactSteps, PublicationError> {
            unreachable!()
        }
    }

    let writes = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let tag = FakeTagPublisher {
        state: Arc::new(Mutex::new(RemoteState::Missing)),
        writes: writes.clone(),
    };
    let publishers = [
        PublisherNode::new(
            "first",
            Box::new(Checked {
                calls: calls.clone(),
                state: PublisherState::Conflicting("conflict".into()),
            }),
            Vec::new(),
        ),
        PublisherNode::new(
            "second",
            Box::new(Checked {
                calls: calls.clone(),
                state: PublisherState::Missing,
            }),
            Vec::new(),
        ),
    ];

    publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        false,
        &mut |_| {},
    )
    .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(writes.lock().unwrap().is_empty());
}

struct CountingPublisher {
    name: String,
    published: AtomicBool,
    checking: Arc<AtomicUsize>,
    max_checking: Arc<AtomicUsize>,
    publishing: Arc<AtomicUsize>,
    max_publishing: Arc<AtomicUsize>,
}

impl Publisher for CountingPublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        if self.published.load(Ordering::SeqCst) {
            return Ok(PublisherState::Published);
        }
        count_concurrency(&self.checking, &self.max_checking);
        Ok(PublisherState::Missing)
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        count_concurrency(&self.publishing, &self.max_publishing);
        self.published.store(true, Ordering::SeqCst);
        let step = ArtifactStep {
            kind: ArtifactKind::Asset,
            name: self.name.clone(),
            step: AssetStep::Uploaded,
        };
        progress(&step);
        Ok(vec![step])
    }
}

fn count_concurrency(active: &AtomicUsize, maximum: &AtomicUsize) {
    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
    maximum.fetch_max(current, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(20));
    active.fetch_sub(1, Ordering::SeqCst);
}

#[test]
fn jobs_limits_checks_and_publishers_and_keeps_the_report_in_declaration_order() {
    let checking = Arc::new(AtomicUsize::new(0));
    let max_checking = Arc::new(AtomicUsize::new(0));
    let publishing = Arc::new(AtomicUsize::new(0));
    let max_publishing = Arc::new(AtomicUsize::new(0));
    let publishers: Vec<_> = (0..4)
        .map(|index| {
            PublisherNode::new(
                format!("publisher.{index}"),
                Box::new(CountingPublisher {
                    name: index.to_string(),
                    published: AtomicBool::new(false),
                    checking: checking.clone(),
                    max_checking: max_checking.clone(),
                    publishing: publishing.clone(),
                    max_publishing: max_publishing.clone(),
                }),
                Vec::new(),
            )
        })
        .collect();

    let (_, outcomes) = publish_ordered_with_jobs(
        &fake_tag(RemoteState::Matching),
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        NonZeroUsize::new(2).unwrap(),
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(max_checking.load(Ordering::SeqCst), 2);
    assert_eq!(max_publishing.load(Ordering::SeqCst), 2);
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| match outcome {
                PublisherOutcome::Published { artifacts, .. } => artifacts[0].name.as_str(),
                PublisherOutcome::Failed { .. } | PublisherOutcome::Skipped { .. } => {
                    panic!("publisher did not complete")
                }
            })
            .collect::<Vec<_>>(),
        ["0", "1", "2", "3"]
    );
}

struct CoordinatedPublisher {
    published: AtomicBool,
    attempts: Arc<AtomicUsize>,
    signal: Option<Arc<AtomicBool>>,
    wait_for: Option<Arc<AtomicBool>>,
    delay: Duration,
    fail: bool,
}

impl Publisher for CoordinatedPublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(if self.published.load(Ordering::SeqCst) {
            PublisherState::Published
        } else {
            PublisherState::Missing
        })
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        _progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if let Some(signal) = &self.signal {
            signal.store(true, Ordering::SeqCst);
        }
        if let Some(wait_for) = &self.wait_for {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !wait_for.load(Ordering::SeqCst) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "publisher did not run concurrently"
                );
                thread::yield_now();
            }
        }
        thread::sleep(self.delay);
        if self.fail {
            return Err(PublicationError::new("failed"));
        }
        self.published.store(true, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

struct CompletionPublisher {
    name: &'static str,
    published: AtomicBool,
    wait_for: Option<Arc<AtomicBool>>,
    completed: Arc<AtomicBool>,
    delay: Duration,
}

impl Publisher for CompletionPublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(if self.published.load(Ordering::SeqCst) {
            PublisherState::Published
        } else {
            PublisherState::Missing
        })
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        if let Some(wait_for) = &self.wait_for {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !wait_for.load(Ordering::SeqCst) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "publisher did not run concurrently"
                );
                thread::yield_now();
            }
        }
        thread::sleep(self.delay);
        self.published.store(true, Ordering::SeqCst);
        self.completed.store(true, Ordering::SeqCst);
        let step = ArtifactStep {
            kind: ArtifactKind::Asset,
            name: self.name.into(),
            step: AssetStep::Uploaded,
        };
        progress(&step);
        Ok(vec![step])
    }
}

#[test]
fn live_events_follow_completion_order_while_the_report_keeps_declaration_order() {
    let fast_completed = Arc::new(AtomicBool::new(false));
    let publishers = [
        PublisherNode::new(
            "slow",
            Box::new(CompletionPublisher {
                name: "slow",
                published: AtomicBool::new(false),
                wait_for: Some(fast_completed.clone()),
                completed: Arc::new(AtomicBool::new(false)),
                delay: Duration::from_millis(20),
            }),
            Vec::new(),
        ),
        PublisherNode::new(
            "fast",
            Box::new(CompletionPublisher {
                name: "fast",
                published: AtomicBool::new(false),
                wait_for: None,
                completed: fast_completed,
                delay: Duration::ZERO,
            }),
            Vec::new(),
        ),
    ];
    let completed = Arc::new(Mutex::new(Vec::new()));
    let progress = completed.clone();

    let (_, outcomes) = publish_ordered_with_jobs(
        &fake_tag(RemoteState::Matching),
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        NonZeroUsize::new(2).unwrap(),
        false,
        &mut move |event| {
            if let PublishEvent::Publisher { name, .. } = event {
                progress.lock().unwrap().push(name.to_owned());
            }
        },
    )
    .unwrap();

    assert_eq!(*completed.lock().unwrap(), ["fast", "slow"]);
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| match outcome {
                PublisherOutcome::Published { artifacts, .. } => artifacts[0].name.as_str(),
                PublisherOutcome::Failed { .. } | PublisherOutcome::Skipped { .. } => {
                    panic!("publisher did not complete")
                }
            })
            .collect::<Vec<_>>(),
        ["slow", "fast"]
    );
}

struct PanickingPublisher;

impl Publisher for PanickingPublisher {
    fn state(&self, _release: &Release) -> Result<PublisherState, PublicationError> {
        Ok(PublisherState::Missing)
    }

    fn publish_with_progress(
        &self,
        _release: &Release,
        _progress: &mut dyn FnMut(&ArtifactStep),
    ) -> Result<ArtifactSteps, PublicationError> {
        panic!("boom")
    }
}

#[test]
fn a_panicking_publisher_is_reported_instead_of_hanging_the_scheduler() {
    let publishers = [PublisherNode::new(
        "broken",
        Box::new(PanickingPublisher),
        Vec::new(),
    )];

    let (_, outcomes) = publish_ordered(
        &fake_tag(RemoteState::Matching),
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(
        outcomes,
        [PublisherOutcome::Failed {
            reason: "publisher broken panicked".into(),
        }]
    );
}

#[test]
fn fail_fast_starts_nothing_new_while_running_publishers_finish() {
    let running_started = Arc::new(AtomicBool::new(false));
    let failure_started = Arc::new(AtomicBool::new(false));
    let failed_attempts = Arc::new(AtomicUsize::new(0));
    let running_attempts = Arc::new(AtomicUsize::new(0));
    let queued_attempts = Arc::new(AtomicUsize::new(0));
    let node = |attempts, signal, wait_for, delay, fail| {
        Box::new(CoordinatedPublisher {
            published: AtomicBool::new(false),
            attempts,
            signal,
            wait_for,
            delay,
            fail,
        }) as Box<dyn Publisher>
    };
    let publishers = [
        PublisherNode::new(
            "worker",
            node(
                failed_attempts.clone(),
                Some(failure_started.clone()),
                Some(running_started.clone()),
                Duration::ZERO,
                true,
            ),
            Vec::new(),
        ),
        PublisherNode::new(
            "running",
            node(
                running_attempts.clone(),
                Some(running_started),
                Some(failure_started),
                Duration::from_millis(50),
                false,
            ),
            Vec::new(),
        ),
        PublisherNode::new(
            "queued",
            node(queued_attempts.clone(), None, None, Duration::ZERO, false),
            Vec::new(),
        ),
    ];

    let (_, outcomes) = publish_ordered_with_jobs(
        &fake_tag(RemoteState::Matching),
        &release("0123456789012345678901234567890123456789".into()),
        &publishers,
        NonZeroUsize::new(2).unwrap(),
        true,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(failed_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(running_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(queued_attempts.load(Ordering::SeqCst), 0);
    assert!(matches!(outcomes[0], PublisherOutcome::Failed { .. }));
    assert!(matches!(outcomes[1], PublisherOutcome::Published { .. }));
    assert_eq!(
        outcomes[2],
        PublisherOutcome::Skipped {
            reason: "skipped: fail-fast after worker failed".into(),
        }
    );
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

    let heading = ChangelogHeading::default();

    validate_new_release(
        &candidate,
        &parent,
        Some(&parent),
        true,
        &analysis,
        changelog,
        &heading,
    )
    .unwrap();

    let too_small = Version::new(1, 2, 4);
    assert!(
        validate_new_release(
            &too_small,
            &parent,
            Some(&parent),
            true,
            &analysis,
            "## [1.2.4] - 2026-11-05\n\nFix.\n",
            &heading,
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
            true,
            &analysis,
            "## [1.4.0] - 2026-11-05\n\nJump.\n",
            &heading,
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
            true,
            &analysis,
            "# Changelog\n",
            &heading,
        )
        .unwrap_err()
        .to_string()
        .contains("does not contain")
    );
}

#[test]
fn accepts_semver_prerelease_steps_and_final_successors() {
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
fn a_rejected_tag_push_reports_the_remotes_reason() {
    use std::os::unix::fs::PermissionsExt as _;
    let repository = GitRepo::new();
    let commit = repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    let remote = tempfile::tempdir().unwrap();
    let remote_path = remote.path().to_string_lossy().into_owned();
    repository.git(["init", "--bare", "--quiet", &remote_path]);
    repository.git(["remote", "add", "origin", &remote_path]);
    let hook = remote.path().join("hooks/pre-receive");
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'tags are protected here' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let publisher = GitTagPublisher::new(repository.path(), "origin");

    let error = publisher.publish(&release(commit)).unwrap_err().to_string();

    assert!(error.contains("tags are protected here"), "{error}");
    assert!(error.contains("pre-receive hook declined"), "{error}");
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
    let tag = fake_tag(RemoteState::Matching);
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
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap()
    .unwrap();

    assert_eq!(verified.commit.to_string(), predecessor);
    assert_eq!(*repositories.borrow(), ["old/project"]);
}

#[test]
fn unreadable_predecessors_require_the_exact_trust_and_a_matching_remote_tag() {
    for unreadable in [
        UnreadableDeclaration::UnsupportedSchema(1),
        UnreadableDeclaration::Missing,
    ] {
        unreadable_predecessor_requires_the_exact_trust(unreadable);
    }
}

fn unreadable_predecessor_requires_the_exact_trust(unreadable: UnreadableDeclaration) {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let predecessor = predecessor.parse().unwrap();
    let candidate = candidate.parse().unwrap();
    let current_declaration = declaration("0.1.0", "new/project");
    let unsupported = match unreadable {
        UnreadableDeclaration::UnsupportedSchema(schema_version) => {
            PublicationError::unsupported_predecessor("v0.1.0", predecessor, schema_version)
        }
        UnreadableDeclaration::Missing => {
            PublicationError::undeclared_predecessor("v0.1.0", predecessor)
        }
    };

    let tag = fake_tag(RemoteState::Matching);
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        None,
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();
    let reported = error.as_unreadable_predecessor().unwrap();
    assert_eq!(reported.commit, predecessor);
    assert_eq!(reported.declaration, unreadable);

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
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not match expected"));

    let trusted = TrustedPredecessor {
        tag: "v0.1.0".into(),
        commit: predecessor,
    };
    *tag.state.lock().unwrap() = RemoteState::Missing;
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing from the remote"));

    *tag.state.lock().unwrap() = RemoteState::Matching;
    let verified = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(verified.commit, predecessor);

    let lightweight = LightweightTag(predecessor);
    let verified = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &lightweight,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(verified.commit, predecessor);

    let elsewhere = LightweightTag(candidate);
    let error = verify_predecessor_with(
        repository.path(),
        candidate,
        &Version::new(0, 1, 0),
        &current_declaration,
        &elsewhere,
        Some(&trusted),
        |_| Err(unsupported.clone()),
        |_, _, _| unreachable!("trusted crossings do not decode old publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("lightweight"));
}

/// A remote whose release tag is lightweight and points to the given commit.
struct LightweightTag(Oid);

impl TagPublisher for LightweightTag {
    fn state(&self, _release: &Release) -> Result<RemoteState, PublicationError> {
        Ok(RemoteState::Conflicting(format!(
            "remote tag is lightweight and points to {}",
            self.0
        )))
    }

    fn publish(&self, _release: &Release) -> Result<(), PublicationError> {
        unreachable!("verification does not publish")
    }

    fn lightweight_target(&self, _tag: &str) -> Result<Option<Oid>, PublicationError> {
        Ok(Some(self.0))
    }
}

#[test]
fn an_ordinary_predecessor_requires_an_annotated_tag() {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let current_declaration = declaration("0.1.0", "new/project");
    let previous_declaration = declaration("0.1.0", "old/project");

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &LightweightTag(predecessor.parse().unwrap()),
        None,
        |_| Ok(previous_declaration.clone()),
        |_, _, _| unreachable!("the tag is checked before publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("lightweight"));
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
    let tag = fake_tag(RemoteState::Matching);

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| unreachable!("ancestry is checked before decoding"),
        |_, _, _| unreachable!("ancestry is checked before publishers"),
        |_, _| unreachable!("no OCI image publishers are declared"),
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
    let tag = fake_tag(RemoteState::Matching);

    let error = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        Some(&trusted),
        |_| Ok(previous_declaration.clone()),
        |_, _, _| Ok(GithubReleaseState::Published { prerelease: false }),
        |_, _| unreachable!("no OCI image publishers are declared"),
    )
    .unwrap_err();

    assert!(error.to_string().contains("unnecessary"));
}

#[test]
fn a_predecessor_without_publishers_is_complete_with_its_remote_tag() {
    let repository = GitRepo::new();
    let predecessor = repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("publisher", "new/project\n");
    let candidate = repository.commit("✨ Change publisher");
    let current_declaration = declaration("0.1.0", "new/project");
    let mut previous_declaration = declaration("0.1.0", "old/project");
    previous_declaration.publishers.github_release.clear();
    let tag = fake_tag(RemoteState::Matching);
    let verify = || {
        verify_predecessor_with(
            repository.path(),
            candidate.parse().unwrap(),
            &Version::new(0, 1, 0),
            &current_declaration,
            &tag,
            None,
            |_commit| Ok(previous_declaration.clone()),
            |_, _, _| unreachable!("no GitHub Release publishers are declared"),
            |_, _| unreachable!("no OCI image publishers are declared"),
        )
    };

    assert_eq!(verify().unwrap().unwrap().commit.to_string(), predecessor);

    *tag.state.lock().unwrap() = RemoteState::Missing;
    assert!(
        verify()
            .unwrap_err()
            .to_string()
            .contains("missing from the remote")
    );
}

fn oci_image_predecessor(tags: &[&str], aliases: &[&str]) -> ReleaseDeclaration {
    let mut declaration = declaration("0.1.0", "old/project");
    declaration.publishers.github_release.clear();
    declaration.publishers.oci_image.insert(
        "default".into(),
        OciImagePublisher {
            after: Vec::new(),
            repository: "ghcr.io/owner/project".into(),
            platforms: [("linux/amd64".into(), "/nix/store/abc-image.tar.gz".into())].into(),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            public: true,
            compression: Compression::default(),
            username: Username::Secret("REGISTRY_USERNAME".into()),
            password: Token {
                secret: "REGISTRY_TOKEN".into(),
            },
        },
    );
    declaration
}

fn annotated_index(version: &str) -> Vec<u8> {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[],"annotations":{{"org.opencontainers.image.version":"{version}"}}}}"#
    )
    .into_bytes()
}

/// Verify a predecessor with OCI image publishers against the manifests a fake registry holds by tag.
fn verify_oci_image_predecessor(
    previous_declaration: &ReleaseDeclaration,
    manifests: &[(&str, Vec<u8>)],
) -> (Result<(), PublicationError>, Vec<String>) {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let current_declaration = declaration("0.1.0", "new/project");
    let tag = fake_tag(RemoteState::Matching);
    let mut read = Vec::new();
    let result = verify_predecessor_with(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::new(0, 1, 0),
        &current_declaration,
        &tag,
        None,
        |_commit| Ok(previous_declaration.clone()),
        |_, _, _| unreachable!("no GitHub Release publishers are declared"),
        |publisher, tag| {
            assert_eq!(publisher.repository, "ghcr.io/owner/project");
            read.push(tag.to_owned());
            Ok(manifests
                .iter()
                .find(|(name, _)| *name == tag)
                .map(|(_, manifest)| manifest.clone()))
        },
    )
    .map(|verified| assert!(verified.is_some()));
    (result, read)
}

#[test]
fn a_complete_oci_image_predecessor_passes_and_ignores_aliases() {
    let previous = oci_image_predecessor(&["{version}", "v{version}"], &["latest"]);
    let (result, read) = verify_oci_image_predecessor(
        &previous,
        &[
            ("0.1.0", annotated_index("0.1.0")),
            ("v0.1.0", annotated_index("0.1.0")),
            // An alias that already moved on, or is missing, does not matter.
            ("latest", annotated_index("0.2.0")),
        ],
    );

    result.unwrap();
    assert_eq!(read, ["0.1.0", "v0.1.0"]);
}

#[test]
fn an_incomplete_oci_image_predecessor_blocks_the_next_release() {
    let previous = oci_image_predecessor(&["{version}", "v{version}"], &["latest"]);
    for (manifests, expected) in [
        (
            vec![("0.1.0", annotated_index("0.1.0"))],
            "ghcr.io/owner/project:v0.1.0 (oci-image.default) is incomplete: the tag is missing",
        ),
        (
            vec![
                ("0.1.0", annotated_index("0.1.0")),
                ("v0.1.0", annotated_index("0.0.9")),
            ],
            "the tag points to version 0.0.9",
        ),
        (
            vec![
                ("0.1.0", br#"{"schemaVersion":2,"manifests":[]}"#.to_vec()),
                ("v0.1.0", annotated_index("0.1.0")),
            ],
            "0.1.0 (oci-image.default) is incomplete: the tag does not point to an image index with a version annotation",
        ),
        (
            // A single image manifest, even with the right annotation, is not the release's index.
            vec![
                (
                    "0.1.0",
                    br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[],"annotations":{"org.opencontainers.image.version":"0.1.0"}}"#.to_vec(),
                ),
                ("v0.1.0", annotated_index("0.1.0")),
            ],
            "0.1.0 (oci-image.default) is incomplete: the tag does not point to an image index with a version annotation",
        ),
        (
            // An index needs schema version 2.
            vec![
                (
                    "0.1.0",
                    br#"{"schemaVersion":1,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[],"annotations":{"org.opencontainers.image.version":"0.1.0"}}"#.to_vec(),
                ),
                ("v0.1.0", annotated_index("0.1.0")),
            ],
            "0.1.0 (oci-image.default) is incomplete: the tag does not point to an image index with a version annotation",
        ),
    ] {
        let (result, _) = verify_oci_image_predecessor(&previous, &manifests);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("previous OCI image release"), "{error}");
        assert!(error.contains(expected), "{error}");
    }
}

fn oci_helm_manifest(version: &str, origin: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": { "mediaType": "application/vnd.cncf.helm.config.v1+json" },
        "layers": [{ "mediaType": "application/vnd.cncf.helm.chart.content.v1.tar+gzip" }],
        "annotations": {
            "org.opencontainers.image.version": version,
            "dev.presswerk.origin": origin,
        }
    }))
    .unwrap()
}

#[test]
fn an_oci_helm_predecessor_needs_its_version_tag_and_store_path() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Prepare predecessor");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");
    let current_declaration = declaration("0.1.0", "new/project");
    let mut previous_declaration = declaration("0.1.0", "old/project");
    previous_declaration.publishers.github_release.clear();
    previous_declaration.publishers.oci_helm.insert(
        "default".into(),
        OciHelmPublisher {
            after: Vec::new(),
            repository: "ghcr.io/owner/charts/project".into(),
            chart: "/nix/store/abc-project-0.1.0.tgz".into(),
            public: true,
            username: Username::Secret("REGISTRY_USERNAME".into()),
            password: Token {
                secret: "REGISTRY_TOKEN".into(),
            },
        },
    );
    let tag = fake_tag(RemoteState::Matching);
    let verify = |manifest: Option<Vec<u8>>| {
        verify_predecessor_with_oci_helm(
            repository.path(),
            candidate.parse().unwrap(),
            &Version::new(0, 1, 0),
            &current_declaration,
            &tag,
            None,
            |_commit| Ok(previous_declaration.clone()),
            |_, _, _| unreachable!("no GitHub Release publishers are declared"),
            |_, _| unreachable!("no OCI image publishers are declared"),
            |publisher, read_tag| {
                assert_eq!(publisher.repository, "ghcr.io/owner/charts/project");
                assert_eq!(read_tag, "0.1.0");
                Ok(manifest.clone())
            },
        )
    };

    verify(Some(oci_helm_manifest(
        "0.1.0",
        "/nix/store/abc-project-0.1.0.tgz",
    )))
    .unwrap();
    let missing = verify(None).unwrap_err().to_string();
    assert!(missing.contains("the tag is missing"), "{missing}");
    let conflict = verify(Some(oci_helm_manifest(
        "0.1.0",
        "/nix/store/other-project-0.1.0.tgz",
    )))
    .unwrap_err()
    .to_string();
    assert!(
        conflict.contains("does not point to this release's Helm chart"),
        "{conflict}"
    );
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
    mock_github_api_on("http://127.0.0.1:0", requests)
}

fn mock_github_api_on(
    url: &str,
    requests: Vec<ExpectedRequest>,
) -> (String, thread::JoinHandle<()>) {
    serve_github_api(
        TcpListener::bind(url.trim_start_matches("http://")).unwrap(),
        requests,
    )
}

fn serve_github_api(
    listener: TcpListener,
    requests: Vec<ExpectedRequest>,
) -> (String, thread::JoinHandle<()>) {
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
fn github_publishes_a_matching_custom_body_draft_found_only_in_the_release_listing() {
    let body = "Frozen notes.\n\nInstall 0.1.0";
    let draft = leak(format!(
        r#"[{{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":{}}}]"#,
        serde_json::to_string(body).unwrap()
    ));
    let published = leak(format!(
        r#"{{"id":7,"draft":false,"prerelease":false,"body":{}}}"#,
        serde_json::to_string(body).unwrap()
    ));
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
            response: draft,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: published,
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
            response: published,
            body_contains: None,
        },
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    )
    .with_body("{{ notes }}\nInstall {{ version }}");

    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();

    server.join().unwrap();
}

#[test]
fn github_reports_a_published_release_with_a_different_body_as_already_published() {
    let requests = (0..2)
        .flat_map(|_| {
            [
                get("/repos/owner/project", 200, "{}"),
                get(
                    "/repos/owner/project/releases/tags/v0.1.0",
                    200,
                    r#"{"id":7,"draft":false,"prerelease":false,"body":"Edited by hand"}"#,
                ),
            ]
        })
        .collect();
    let (url, server) = mock_github_api(requests);
    let tag = fake_tag(RemoteState::Matching);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    )
    .with_body("{{ notes }}\nInstall {{ version }}");

    let steps = publish_ordered(
        &tag,
        &release("0123456789012345678901234567890123456789".into()),
        &[PublisherNode::new("test", Box::new(publisher), Vec::new())],
        false,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(
        steps,
        (
            TagStep::AlreadyPushed,
            vec![PublisherOutcome::Published {
                step: PublisherStep::AlreadyPublished,
                artifacts: Vec::new(),
            }]
        )
    );
    server.join().unwrap();
}

#[test]
fn github_rejects_an_existing_draft_with_a_different_custom_body() {
    let (url, server) = mock_github_api(tag_lookup_and_listing(vec![
        r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Different body"}]"#
            .into(),
    ]));
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    )
    .with_body("{{ notes }}\nInstall {{ version }}");

    let error = publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap_err();

    assert!(error.to_string().contains("does not match"), "{error}");
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
                r#"{{"id":{},"tag_name":"v0.0.{index}","draft":false,"prerelease":false,"body":"Frozen notes.\n"}}"#,
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
        r#"[{"id":8,"tag_name":"v0.1.0-rc.1","draft":true,"prerelease":true,"body":"Candidate notes.\n"},{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#.into(),
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
            r#"[{"id":7,"tag_name":"","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#,
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
        r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"},{"id":8,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#.into(),
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
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#,
            body_contains: None,
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n"}"#,
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
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n"}"#,
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
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#,
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
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n"}"#,
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
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n"}"#,
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
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#,
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
            response: r#"[{"id":7,"tag_name":"v0.1.0","draft":true,"prerelease":false,"body":"Frozen notes.\n"}]"#,
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
            response: r#"{"id":7,"draft":true,"prerelease":false,"body":"Frozen notes.\n\nInstall 0.1.0 from v0.1.0"}"#,
            body_contains: Some(r#""body":"Frozen notes.\n\nInstall 0.1.0 from v0.1.0""#),
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n\nInstall 0.1.0 from v0.1.0"}"#,
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
            response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n\nInstall 0.1.0 from v0.1.0"}"#,
            body_contains: None,
        },
    ]);
    let client = GithubClient::new(&url, "test-token").unwrap();
    let publisher = GithubPublisher::new(client, "owner/project")
        .with_body("{{ notes }}\nInstall {{ version }} from {{ tag }}");

    publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();

    server.join().unwrap();
}

#[test]
fn github_links_the_changelog_when_the_body_exceeds_the_limit() {
    const FALLBACK: &str = "The release notes are too long for a GitHub Release; read them in [docs/CHANGES.md](https://github.com/owner/project/blob/v0.1.0/docs/CHANGES.md).";
    let repository = r#"{"html_url":"https://github.com/owner/project"}"#;
    let created = leak(format!(
        r#"{{"id":7,"draft":true,"prerelease":false,"body":{}}}"#,
        serde_json::to_string(FALLBACK).unwrap()
    ));
    let published = leak(created.replace(r#""draft":true"#, r#""draft":false"#));
    let (url, server) = mock_github_api(vec![
        get("/repos/owner/project", 200, repository),
        get("/repos/owner/project", 200, "{}"),
        get("/repos/owner/project/releases/tags/v0.1.0", 404, "{}"),
        get(
            "/repos/owner/project/releases?per_page=100&page=1",
            200,
            "[]",
        ),
        ExpectedRequest {
            method: "POST",
            path: "/repos/owner/project/releases",
            status: 201,
            response: created,
            body_contains: Some(leak(format!(
                r#""body":{}"#,
                serde_json::to_string(FALLBACK).unwrap()
            ))),
        },
        ExpectedRequest {
            method: "PATCH",
            path: "/repos/owner/project/releases/7",
            status: 200,
            response: published,
            body_contains: Some(r#""draft":false"#),
        },
        get("/repos/owner/project", 200, repository),
        get("/repos/owner/project", 200, "{}"),
        get("/repos/owner/project/releases/tags/v0.1.0", 200, published),
    ]);
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    )
    .with_changelog("docs/CHANGES.md");
    let mut release = release("0123456789012345678901234567890123456789".into());
    release.notes = "x".repeat(GITHUB_RELEASE_BODY_LIMIT + 1);

    publisher.publish(&release).unwrap();

    server.join().unwrap();
}

#[test]
fn github_body_template_errors_before_reading_remote_state() {
    let publisher = GithubPublisher::new(
        GithubClient::new("https://api.github.com", "test-token").unwrap(),
        "owner/project",
    )
    .with_body("{{ unknown }}");

    let error = publisher
        .state(&release("0123456789012345678901234567890123456789".into()))
        .unwrap_err();

    assert!(error.to_string().contains("body template"), "{error}");
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn get(path: &'static str, status: u16, response: &'static str) -> ExpectedRequest {
    ExpectedRequest {
        method: "GET",
        path,
        status,
        response,
        body_contains: None,
    }
}

fn local_asset(directory: &tempfile::TempDir) -> LocalAsset {
    let path = directory.path().join("tool");
    std::fs::write(&path, "tool\n").unwrap();
    LocalAsset::from_file("tool-x86_64-linux", path).unwrap()
}

/// Bind the mock first, so responses can point GitHub's upload URL at it.
fn mock_github_api_at(
    requests: impl FnOnce(&str) -> Vec<ExpectedRequest>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = requests(&url);
    // Keep the listener: binding the port again could race with another test.
    let (actual, handle) = serve_github_api(listener, requests);
    assert_eq!(actual, url);
    (url, handle)
}

#[test]
fn github_uploads_assets_into_the_draft_before_publishing_it() {
    let directory = tempfile::tempdir().unwrap();
    let asset = local_asset(&directory);
    let (url, server) = mock_github_api_at(|url| {
        let draft = leak(format!(
            r#"{{"id":7,"draft":true,"prerelease":false,"body":"Frozen notes.\n","upload_url":"{url}/uploads/7/assets{{?name,label}}","assets":[]}}"#
        ));
        let published = leak(format!(
            r#"{{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[{{"name":"tool-x86_64-linux","digest":"{}"}}]}}"#,
            asset.digest
        ));
        vec![
            get("/repos/owner/project", 200, "{}"),
            get("/repos/owner/project/releases/tags/v0.1.0", 404, "{}"),
            get(
                "/repos/owner/project/releases?per_page=100&page=1",
                200,
                "[]",
            ),
            ExpectedRequest {
                method: "POST",
                path: "/repos/owner/project/releases",
                status: 201,
                response: draft,
                body_contains: Some(r#""body":"Frozen notes.\n""#),
            },
            ExpectedRequest {
                method: "POST",
                path: "/uploads/7/assets?name=tool-x86_64-linux",
                status: 201,
                response: "{}",
                body_contains: Some("tool\n"),
            },
            ExpectedRequest {
                method: "PATCH",
                path: "/repos/owner/project/releases/7",
                status: 200,
                response: r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n"}"#,
                body_contains: Some(r#""draft":false"#),
            },
            get("/repos/owner/project", 200, "{}"),
            get("/repos/owner/project/releases/tags/v0.1.0", 200, published),
        ]
    });
    let publisher = GithubPublisher::new(
        GithubClient::new(&url, "test-token").unwrap(),
        "owner/project",
    )
    .with_assets(vec![asset]);

    let steps = publisher
        .publish(&release("0123456789012345678901234567890123456789".into()))
        .unwrap();

    assert_eq!(
        steps,
        [ArtifactStep {
            kind: ArtifactKind::Asset,
            name: "tool-x86_64-linux".to_owned(),
            step: AssetStep::Uploaded,
        }]
    );
    server.join().unwrap();
}

#[test]
fn github_assets_decide_whether_a_published_release_is_complete() {
    let directory = tempfile::tempdir().unwrap();
    let asset = local_asset(&directory);
    let release = release("0123456789012345678901234567890123456789".into());
    let matching = leak(format!(
        r#"{{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[{{"name":"tool-x86_64-linux","digest":"{}"}}]}}"#,
        asset.digest
    ));
    for (response, expected) in [
        (
            r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[]}"#,
            Ok(PublisherState::Incomplete),
        ),
        (matching, Ok(PublisherState::Published)),
        (
            r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[{"name":"tool-x86_64-linux","digest":"sha256:00"}]}"#,
            Err("has digest sha256:00"),
        ),
        (
            r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[{"name":"tool-x86_64-linux"}]}"#,
            Err("has no digest"),
        ),
        (
            r#"{"id":7,"draft":false,"prerelease":false,"body":"Frozen notes.\n","assets":[{"name":"by-hand.txt","digest":"sha256:00"}]}"#,
            Ok(PublisherState::Incomplete),
        ),
    ] {
        let (url, server) = mock_github_api(vec![
            get("/repos/owner/project", 200, "{}"),
            get("/repos/owner/project/releases/tags/v0.1.0", 200, response),
        ]);
        let publisher = GithubPublisher::new(
            GithubClient::new(&url, "test-token").unwrap(),
            "owner/project",
        )
        .with_assets(vec![asset.clone()]);

        match (publisher.state(&release).unwrap(), expected) {
            (PublisherState::Conflicting(message), Err(expected)) => {
                assert!(message.contains(expected), "{message}");
            }
            (state, Ok(expected)) => assert_eq!(state, expected),
            (state, Err(expected)) => {
                panic!("expected a conflict with {expected:?}, got {state:?}")
            }
        }
        server.join().unwrap();
    }
}

#[test]
fn a_github_release_asset_must_be_a_regular_file() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        LocalAsset::from_file("tool", directory.path())
            .unwrap_err()
            .to_string()
            .contains("is not a regular file")
    );
}

/// Reads the declaration of a `commit_version` repository, whose `VERSION` file is the version.
fn version_file_declaration(repository: &GitRepo, commit: Oid) -> Option<ReleaseDeclaration> {
    let version = repository.version_at(&commit.to_string());
    Some(declaration(&version, "owner/project"))
}

fn find_pending(
    repository: &GitRepo,
    candidate: &str,
    version: &str,
    tag: &FakeTagPublisher,
) -> Result<Option<UntaggedRelease>, PublicationError> {
    pending_predecessor(
        repository.path(),
        candidate.parse().unwrap(),
        &Version::parse(version).unwrap(),
        &declaration(version, "owner/project"),
        tag,
        |commit| Ok(version_file_declaration(repository, commit)),
    )
}

#[test]
fn finds_the_pending_release_below_several_commits() {
    let repository = GitRepo::new();
    repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    repository.annotated_tag("v0.1.0");
    repository.write("feature", "implemented\n");
    repository.commit("✨ Add a feature");
    let pending = repository.commit_version("0.2.0", "🔖 Release v0.2.0");
    let missing = fake_tag(RemoteState::Missing);

    let Some(UntaggedRelease::Pending(at_head)) =
        find_pending(&repository, &pending, "0.2.0", &missing).unwrap()
    else {
        panic!("the release commit is pending");
    };
    assert_eq!(at_head.commit.to_string(), pending);
    assert_eq!(at_head.tag, "v0.2.0");
    assert!(at_head.pending);

    repository.write("fix", "fixed\n");
    repository.commit("🐛 Fix a bug");
    repository.write("feature", "extended\n");
    let candidate = repository.commit("✨ Extend the feature");
    let Some(UntaggedRelease::Pending(below)) =
        find_pending(&repository, &candidate, "0.2.0", &missing).unwrap()
    else {
        panic!("the release commit is pending");
    };
    assert_eq!(below.commit.to_string(), pending);
    assert_eq!(below.version, Version::new(0, 2, 0));
    assert!(below.pending);
}

#[test]
fn a_release_with_a_tag_is_not_pending() {
    let repository = GitRepo::new();
    repository.commit_version("0.0.0", "🎉 Begin project");
    let release = repository.commit_version("0.1.0", "🔖 Release v0.1.0");
    repository.write("feature", "implemented\n");
    let candidate = repository.commit("✨ Add a feature");

    // Published, but the tag was not fetched: the remote tag stands in for it.
    let published = fake_tag(RemoteState::Matching);
    assert_eq!(
        find_pending(&repository, &candidate, "0.1.0", &published).unwrap(),
        Some(UntaggedRelease::Published(release.parse().unwrap()))
    );

    let conflicting = fake_tag(RemoteState::Conflicting("tag points elsewhere".into()));
    let error = find_pending(&repository, &candidate, "0.1.0", &conflicting).unwrap_err();
    assert_eq!(error.to_string(), "tag points elsewhere");

    repository.git([
        "tag",
        "--annotate",
        "--message",
        "v0.1.0",
        "v0.1.0",
        &release,
    ]);
    let missing = fake_tag(RemoteState::Missing);
    assert_eq!(
        find_pending(&repository, &candidate, "0.1.0", &missing).unwrap(),
        None
    );
}

#[test]
fn neither_the_baseline_nor_a_root_commit_is_a_pending_release() {
    let missing = fake_tag(RemoteState::Missing);

    let baseline = GitRepo::new();
    let root = baseline.commit_version("0.0.0", "🎉 Begin project");
    baseline.write("feature", "implemented\n");
    let candidate = baseline.commit("✨ Add a feature");
    assert_eq!(
        find_pending(&baseline, &candidate, "0.0.0", &missing).unwrap(),
        None
    );
    assert_eq!(
        find_pending(&baseline, &root, "0.0.0", &missing).unwrap(),
        None
    );

    let started_later = GitRepo::new();
    started_later.commit_version("0.3.0", "🎉 Begin project");
    started_later.write("feature", "implemented\n");
    let candidate = started_later.commit("✨ Add a feature");
    assert_eq!(
        find_pending(&started_later, &candidate, "0.3.0", &missing).unwrap(),
        None
    );
}
