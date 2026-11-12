use anyhow::Result;
use clap::{CommandFactory, Parser};
use presswerk::analysis::GitCliffAnalyzer;
use presswerk::cli::{Cli, Command};
use presswerk::preparation::{
    build_head_release_plan, build_head_release_plan_with_trust, is_worktree_dirty,
    prepare_release, render_plan,
};
use presswerk::publication::{
    ArtifactKind, AssetStep, PublishOutcome, PublisherStep, TagStep, publish,
};
use presswerk::release_commit::{TrustedPredecessor, release_subject};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };

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
                        release_subject(tag),
                    ]);
                    print!("{}", render_steps(&rows));
                }
                None => println!("Nothing to prepare."),
            }
        }
        Command::Publish { revision } => match publish(".", &revision)? {
            PublishOutcome::NothingToDo { commit } => println!(
                "Nothing to publish: {} is not a release commit.",
                short_commit(&commit)
            ),
            PublishOutcome::Published(report) => {
                let commit = short_commit(&report.commit);
                println!("Publishing {} from {commit}", report.tag);
                let tag_status = match report.tag_step {
                    TagStep::Pushed => "pushed",
                    TagStep::AlreadyPushed => "already pushed",
                };
                let mut rows = vec![["tag".into(), report.tag.clone(), tag_status.into()]];
                for (name, step, artifacts) in &report.publisher_steps {
                    let oci = name.starts_with("oci.");
                    let status = match step {
                        PublisherStep::Published => "published",
                        PublisherStep::DraftCompleted => "draft completed",
                        PublisherStep::ArtifactsCompleted if oci => "tags completed",
                        PublisherStep::ArtifactsCompleted => "assets completed",
                        PublisherStep::AlreadyPublished => "already published",
                    };
                    let kind = if oci { "registry" } else { "release" };
                    rows.push([kind.into(), name.clone(), status.into()]);
                    rows.extend(artifacts.iter().map(|artifact| {
                        let (kind, done, already) = match artifact.kind {
                            ArtifactKind::Asset => ("asset", "uploaded", "already uploaded"),
                            ArtifactKind::Image => ("image", "pushed", "already pushed"),
                            ArtifactKind::Tag => ("tag", "pushed", "already pushed"),
                            ArtifactKind::Alias => ("alias", "set", "already set"),
                        };
                        let status = match &artifact.step {
                            AssetStep::Uploaded => done.to_owned(),
                            AssetStep::AlreadyUploaded => already.to_owned(),
                            AssetStep::Kept(reason) => format!("kept ({reason})"),
                        };
                        [kind.into(), artifact.name.clone(), status]
                    }));
                }
                print!("{}", render_steps(&rows));
                if report.changed_nothing() {
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
    Ok(())
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
