use clap::{CommandFactory, Parser};

#[derive(Parser)]
#[command(version, about)]
struct Cli;

fn main() {
    if std::env::args_os().len() == 1 {
        Cli::command().print_help().expect("failed to print help");
        println!();
    } else {
        Cli::parse();
    }
}
