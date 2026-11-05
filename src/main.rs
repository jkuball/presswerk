use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use presswerk::publication::{PublishOutcome, publish};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
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
        Command::Publish { revision } => match publish(".", &revision)? {
            PublishOutcome::NothingToDo => println!("Nothing to do."),
            PublishOutcome::Published => println!("Released."),
        },
    }
    Ok(())
}
