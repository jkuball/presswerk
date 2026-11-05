use anyhow::{Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use presswerk::analysis::GitCliffAnalyzer;
use presswerk::preparation::{
    build_head_release_plan, is_worktree_dirty, prepare_release, render_plan,
};
use presswerk::publication::{PublishOutcome, publish};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show the release that would be prepared without writing anything.
    Plan,
    /// Create a release commit from the committed HEAD tree.
    Prepare,
    /// Publish an exact release candidate.
    Publish {
        #[arg(default_value = "HEAD")]
        revision: String,
    },
}

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
        Command::Prepare => {
            if is_worktree_dirty(".")? {
                bail!("working tree is dirty; commit or stash your changes first");
            }
            let plan = build_head_release_plan(".", &GitCliffAnalyzer)?;
            match prepare_release(&plan)? {
                Some(commit) => println!(
                    "committed  {}  🔖 Release v{}",
                    short_commit(&commit.to_string()),
                    plan.next_version.as_ref().expect("release version exists")
                ),
                None => println!("Nothing to prepare."),
            }
        }
        Command::Publish { revision } => match publish(".", &revision)? {
            PublishOutcome::NothingToDo => println!("Nothing to do."),
            PublishOutcome::Published => println!("Released."),
        },
    }
    Ok(())
}

fn short_commit(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}
