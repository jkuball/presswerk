use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "presswerk", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show the release that would be prepared without writing anything.
    Plan,
    /// Create a release commit from the committed HEAD tree.
    Prepare {
        /// Trust one exact unsupported predecessor after verifying it with its original toolchain.
        #[arg(long, value_name = "TAG=SHA")]
        trust_predecessor: Option<String>,
    },
    /// Publish an exact release candidate.
    Publish {
        /// Revision of the release candidate to publish.
        #[arg(default_value = "HEAD")]
        revision: String,
    },
}

/// A command of the CLI reference, generated from the clap definitions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandReference {
    pub name: String,
    pub about: Option<String>,
    pub usage: String,
    pub arguments: Vec<ArgumentReference>,
    pub options: Vec<ArgumentReference>,
    pub subcommands: Vec<CommandReference>,
}

/// A positional argument or option of a command in the CLI reference.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArgumentReference {
    pub id: String,
    pub long: Option<String>,
    pub short: Option<char>,
    pub value_names: Vec<String>,
    pub required: bool,
    pub defaults: Vec<String>,
    pub help: Option<String>,
}

/// Describe every command of the CLI for the generated documentation.
pub fn reference() -> CommandReference {
    let mut command = Cli::command();
    command.build();
    command_reference(&mut command)
}

fn command_reference(command: &mut clap::Command) -> CommandReference {
    let usage = command.render_usage().to_string();
    let usage = usage.strip_prefix("Usage: ").unwrap_or(&usage).to_owned();
    let (arguments, options) = command
        .get_arguments()
        .filter(|argument| {
            !matches!(
                argument.get_action(),
                ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong
            )
        })
        .map(argument_reference)
        .partition(|argument| argument.long.is_none() && argument.short.is_none());
    CommandReference {
        name: command
            .get_bin_name()
            .unwrap_or(command.get_name())
            .to_owned(),
        about: command.get_about().map(ToString::to_string),
        usage,
        arguments,
        options,
        subcommands: command
            .get_subcommands_mut()
            .filter(|subcommand| !subcommand.is_hide_set() && subcommand.get_name() != "help")
            .map(command_reference)
            .collect(),
    }
}

fn argument_reference(argument: &clap::Arg) -> ArgumentReference {
    let takes_value = argument
        .get_num_args()
        .is_some_and(|range| range.takes_values());
    ArgumentReference {
        id: argument.get_id().to_string(),
        long: argument.get_long().map(ToOwned::to_owned),
        short: argument.get_short(),
        value_names: if takes_value {
            argument
                .get_value_names()
                .map(|names| names.iter().map(ToString::to_string).collect())
                .unwrap_or_else(|| vec![argument.get_id().to_string().to_uppercase()])
        } else {
            Vec::new()
        },
        required: argument.is_required_set(),
        defaults: argument
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        help: argument.get_help().map(ToString::to_string),
    }
}
