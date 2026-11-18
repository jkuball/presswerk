use anyhow::Result;
use clap::{CommandFactory, Parser};
use presswerk::analysis::GitCliffAnalyzer;
use presswerk::cli::{Cli, Command};
use presswerk::preparation::{
    ReleasePlan, build_head_release_plan, build_head_release_plan_with_trust, is_worktree_dirty,
    prepare_release, render_plan,
};
use presswerk::publication::{
    ArtifactKind, AssetStep, PublishEvent, PublishOutcome, PublisherStep, TagStep, publish,
};
use presswerk::release_commit::TrustedPredecessor;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(true);
    };

    let mut successful = true;
    match command {
        Command::Plan => {
            let dirty = is_worktree_dirty(".")?;
            let plan = build_head_release_plan(".", &GitCliffAnalyzer)?;
            if dirty {
                eprintln!(
                    "warning: working tree is dirty; showing the plan for HEAD ({}) without local changes",
                    short_commit(&plan.commit.to_string())
                );
            }
            print_alias_notes(&plan);
            print!("{}", render_plan(&plan));
        }
        Command::Prepare { trust_predecessor } => {
            let dirty = is_worktree_dirty(".")?;
            let trusted_predecessor = trust_predecessor
                .as_deref()
                .map(TrustedPredecessor::from_argument)
                .transpose()?;
            let plan =
                build_head_release_plan_with_trust(".", &GitCliffAnalyzer, trusted_predecessor)?;
            if dirty {
                eprintln!(
                    "warning: working tree is dirty; preparing HEAD ({}) without local changes",
                    short_commit(&plan.commit.to_string())
                );
            }
            print_alias_notes(&plan);
            match prepare_release(&plan)? {
                Some(commit) => {
                    let version = plan.next_version.as_ref().expect("release version exists");
                    let tag = plan.next_tag.as_ref().expect("release tag exists");
                    println!(
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
                    print!("{}", render_steps(&rows));
                }
                None => println!("Nothing to prepare."),
            }
        }
        Command::Publish {
            revision,
            fail_fast,
        } => match publish(".", &revision, fail_fast, &mut print_publish_event)? {
            PublishOutcome::NothingToDo { commit } => println!(
                "Nothing to publish: {} is not a release commit.",
                short_commit(&commit)
            ),
            PublishOutcome::Published(report) => {
                let commit = short_commit(&report.commit);
                let warnings = report.warnings();
                if !warnings.is_empty() {
                    eprintln!(
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
                    println!(
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
                    println!(
                        "{} from {commit} was already released; nothing changed.",
                        report.tag
                    );
                } else {
                    println!("Released {} from {commit}.", report.tag);
                }
            }
        },
    }
    Ok(successful)
}

/// Tells before anything is written which aliases another tool pushed: `publish` keeps them, or
/// takes them over from the trusted predecessor.
fn print_alias_notes(plan: &ReleasePlan) {
    for (publisher, variables) in &plan.anonymous_alias_reads {
        let verb = if variables.len() == 1 { "is" } else { "are" };
        eprintln!(
            "warning: {publisher} is not marked public and {} {verb} not set; checking its aliases anonymously, which will probably fail",
            variables.join("/")
        );
    }
    for alias in &plan.foreign_aliases {
        match &alias.predecessor_tag {
            Some(tag) => eprintln!(
                "note: alias {} points to the trusted predecessor's tag {tag}; publish will take it over",
                alias.reference
            ),
            None => eprintln!(
                "warning: alias {} points to {} without a version annotation, so another tool pushed it; publish will keep it",
                alias.reference, alias.digest
            ),
        }
    }
    for (publisher, reason) in &plan.unchecked_aliases {
        eprintln!("warning: could not check the aliases of {publisher}: {reason}");
    }
}

/// Prints each step of `publish` as soon as it completes. Kind and status come from fixed sets,
/// so their columns align without knowing the names that follow.
fn print_publish_event(event: PublishEvent<'_>) {
    let (kind, status, target) = match event {
        PublishEvent::Started { tag, commit } => {
            println!("Publishing {tag} from {}", short_commit(commit));
            return;
        }
        PublishEvent::Tag { tag, step } => {
            let status = match step {
                TagStep::Pushed => "pushed",
                TagStep::AlreadyPushed => "already pushed",
            };
            ("tag", status, tag.to_owned())
        }
        PublishEvent::Publisher { name, step } => {
            let registry = name.starts_with("oci-image.") || name.starts_with("oci-helm.");
            let status = match step {
                PublisherStep::Published => "published",
                PublisherStep::DraftCompleted => "draft completed",
                PublisherStep::ArtifactsCompleted if registry => "tags completed",
                PublisherStep::ArtifactsCompleted => "assets completed",
                PublisherStep::AlreadyPublished => "already published",
            };
            let kind = if registry { "registry" } else { "release" };
            (kind, status, name.to_owned())
        }
        PublishEvent::PublisherFailed { name, reason } => {
            ("publisher", "failed", format!("{name} ({reason})"))
        }
        PublishEvent::PublisherSkipped { name, reason } => (
            "publisher",
            "skipped",
            format!(
                "{name} ({})",
                reason.strip_prefix("skipped: ").unwrap_or(reason)
            ),
        ),
        PublishEvent::Artifact(artifact) => {
            let (kind, done, already) = match artifact.kind {
                ArtifactKind::Asset => ("asset", "uploaded", "already uploaded"),
                ArtifactKind::Image => ("image", "pushed", "already pushed"),
                ArtifactKind::Tag => ("tag", "pushed", "already pushed"),
                ArtifactKind::Alias => ("alias", "set", "already set"),
            };
            let name = &artifact.name;
            match &artifact.step {
                AssetStep::Uploaded => (kind, done, name.clone()),
                AssetStep::AlreadyUploaded => (kind, already, name.clone()),
                AssetStep::TakenOver => (kind, "taken over", name.clone()),
                AssetStep::Kept(reason) => (kind, "kept", format!("{name} ({reason})")),
                AssetStep::Warning(reason) => (
                    kind,
                    "not moved",
                    format!("{name} ({reason}; rerun to repair)"),
                ),
            }
        }
    };
    println!("  {kind:<9}  {status:<17}  {target}");
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
