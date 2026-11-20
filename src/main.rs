use anyhow::Result;
use clap::{CommandFactory, Parser};
use presswerk::analysis::GitCliffAnalyzer;
use presswerk::cli::{Cli, Command};
use presswerk::preparation::{
    ReleasePlan, build_head_preparation_plan, build_head_release_plan, is_worktree_dirty,
    prepare_release, render_plan,
};
use presswerk::publication::{
    AssetStep, PublishEvent, PublishOutcome, PublisherStep, TagStep, publish,
};
use presswerk::publishers::shared::secrets::redact;
use presswerk::release_commit::TrustedPredecessor;

// Everything the CLI prints goes through these, so no secret value it read ever reaches the
// output, whichever part of the output it would have ended up in.
macro_rules! out {
    ($($argument:tt)*) => {
        print!("{}", redact(&format!($($argument)*)))
    };
}

macro_rules! outln {
    () => {
        println!()
    };
    ($($argument:tt)*) => {
        println!("{}", redact(&format!($($argument)*)))
    };
}

macro_rules! errln {
    ($($argument:tt)*) => {
        eprintln!("{}", redact(&format!($($argument)*)))
    };
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            errln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        outln!();
        return Ok(true);
    };

    let mut successful = true;
    match command {
        Command::Plan => {
            let dirty = is_worktree_dirty(".")?;
            let plan = build_head_release_plan(".", &GitCliffAnalyzer)?;
            if dirty {
                errln!(
                    "warning: working tree is dirty; showing the plan for HEAD ({}) without local changes",
                    short_commit(&plan.commit.to_string())
                );
            }
            print_warnings(&plan);
            out!("{}", render_plan(&plan));
        }
        Command::Prepare { trust_predecessor } => {
            let dirty = is_worktree_dirty(".")?;
            let trusted_predecessor = trust_predecessor
                .as_deref()
                .map(TrustedPredecessor::from_argument)
                .transpose()?;
            let plan = build_head_preparation_plan(".", &GitCliffAnalyzer, trusted_predecessor)?;
            if dirty {
                errln!(
                    "warning: working tree is dirty; preparing HEAD ({}) without local changes",
                    short_commit(&plan.commit.to_string())
                );
            }
            print_warnings(&plan);
            match prepare_release(&plan)? {
                Some(commit) => {
                    let version = plan.next_version.as_ref().expect("release version exists");
                    let tag = plan.next_tag.as_ref().expect("release tag exists");
                    outln!(
                        "Preparing {tag} on {}",
                        short_commit(&plan.commit.to_string())
                    );
                    let mut rows: Vec<[String; 3]> = plan
                        .version_files
                        .iter()
                        .map(|file| {
                            [
                                "version".into(),
                                file.path.clone(),
                                format!("{} -> {}", file.current, file.next),
                            ]
                        })
                        .collect();
                    rows.push([
                        "notes".into(),
                        plan.changelog_path.clone(),
                        format!("added {version}"),
                    ]);
                    rows.push([
                        "commit".into(),
                        short_commit(&commit.to_string()).into(),
                        plan.release_subject
                            .clone()
                            .expect("release subject exists"),
                    ]);
                    out!("{}", render_steps(&rows));
                }
                None => match plan.pending_release() {
                    Some(pending) => outln!(
                        "Nothing to prepare: pending release {} at {} is not published yet.",
                        pending.tag,
                        short_commit(&pending.commit.to_string())
                    ),
                    None => outln!("Nothing to prepare."),
                },
            }
        }
        Command::Publish {
            revision,
            fail_fast,
            jobs,
        } => match publish(".", &revision, jobs, fail_fast, &mut print_publish_event)? {
            PublishOutcome::NothingToDo { commit } => outln!(
                "Nothing to publish: {} is not a release commit.",
                short_commit(&commit)
            ),
            PublishOutcome::Published(report) => {
                let commit = short_commit(&report.commit);
                let warnings = report.warnings();
                if !warnings.is_empty() {
                    errln!(
                        "warning: {} could not be updated; {} is complete, publish it again to repair",
                        warnings
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(", "),
                        report.tag
                    );
                }
                if !report.complete() {
                    let failed = report.failed();
                    let skipped = report.skipped();
                    outln!(
                        "Published {} from {commit} partially; {failed} {}, {skipped} skipped.",
                        report.tag,
                        if failed == 1 {
                            "publisher failed"
                        } else {
                            "publishers failed"
                        }
                    );
                    successful = false;
                } else if report.changed_nothing() {
                    outln!(
                        "{} from {commit} was already released; nothing changed.",
                        report.tag
                    );
                } else {
                    outln!("Released {} from {commit}.", report.tag);
                }
            }
        },
    }
    Ok(successful)
}

/// Tells before anything is written what the publishers found on their remotes, such as the
/// aliases another tool pushed: `publish` keeps them, or takes them over from the trusted
/// predecessor.
fn print_warnings(plan: &ReleasePlan) {
    for warning in &plan.warnings {
        errln!("{warning}");
    }
}

/// Prints each step of `publish` as soon as it completes. Kind and status come from fixed sets,
/// so their columns align without knowing the names that follow.
fn print_publish_event(event: PublishEvent<'_>) {
    let (kind, status, target) = match event {
        PublishEvent::Started { tag, commit } => {
            outln!("Publishing {tag} from {}", short_commit(commit));
            return;
        }
        PublishEvent::Tag { tag, step } => {
            let status = match step {
                TagStep::Pushed => "pushed",
                TagStep::AlreadyPushed => "already pushed",
            };
            ("tag".to_owned(), status.to_owned(), tag.to_owned())
        }
        PublishEvent::Publisher { name, labels, step } => {
            let status = match step {
                PublisherStep::Published => "published".to_owned(),
                PublisherStep::DraftCompleted => "draft completed".to_owned(),
                PublisherStep::ArtifactsCompleted => format!("{} completed", labels.artifacts),
                PublisherStep::AlreadyPublished => "already published".to_owned(),
            };
            (labels.publisher.to_owned(), status, name.to_owned())
        }
        PublishEvent::PublisherFailed { name, reason } => (
            "publisher".to_owned(),
            "failed".to_owned(),
            format!("{name} ({reason})"),
        ),
        PublishEvent::PublisherSkipped { name, reason } => (
            "publisher".to_owned(),
            "skipped".to_owned(),
            format!(
                "{name} ({})",
                reason.strip_prefix("skipped: ").unwrap_or(reason)
            ),
        ),
        PublishEvent::Artifact(artifact) => {
            let kind = artifact.kind.label.to_string();
            let verb = &artifact.kind.verb;
            let name = &artifact.name;
            match &artifact.step {
                AssetStep::Uploaded => (kind, verb.to_string(), name.clone()),
                AssetStep::AlreadyUploaded => (kind, format!("already {verb}"), name.clone()),
                AssetStep::TakenOver => (kind, "taken over".to_owned(), name.clone()),
                AssetStep::Kept(reason) => (kind, "kept".to_owned(), format!("{name} ({reason})")),
                AssetStep::Warning(reason) => (
                    kind,
                    "not moved".to_owned(),
                    format!("{name} ({reason}; rerun to repair)"),
                ),
            }
        }
    };
    outln!("  {kind:<9}  {status:<17}  {target}");
}

/// Renders one indented line per step with aligned kind and target columns.
fn render_steps(rows: &[[String; 3]]) -> String {
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (kind_width, target_width) = (width(0), width(1));
    rows.iter()
        .map(|[kind, target, status]| {
            format!("  {kind:<kind_width$}  {target:<target_width$}  {status}\n")
        })
        .collect()
}

fn short_commit(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}
