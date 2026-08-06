use super::shared::{
    command_with_shared_args, no_sandbox_from_env, parse_command_matches,
    parse_shared_command_args, validate_in_place_options,
};
use crate::check::cli_args::value_arg;
use crate::check::core::AskCommandArgs;
use crate::notes::arg_to_string;
use clap::Command;
use std::ffi::OsString;

pub(crate) fn parse_ask_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<AskCommandArgs, String> {
    let matches = parse_command_matches(ask_help_command(), args)?;
    // Docker selects external isolation for every evaluator command through
    // the environment without expanding `canon ask`'s pre-refactor CLI.
    let no_sandbox = no_sandbox_from_env()?;
    let shared = parse_shared_command_args(&matches, default_in_place, no_sandbox)?;
    let question = match matches.get_one::<OsString>("question") {
        Some(value) => arg_to_string(value)?,
        None => return Err("question is required".to_string()),
    };
    let default_agent_preset = match matches.get_one::<OsString>("preset") {
        Some(value) => {
            let value = arg_to_string(value)?;
            if value.trim().is_empty() {
                return Err("--preset name must not be empty".to_string());
            }
            Some(value)
        }
        None => None,
    };

    if shared.in_place {
        validate_in_place_options(
            "canon ask",
            shared.tree_explicit,
            shared.against_tree_explicit,
        )?;
    }

    Ok(AskCommandArgs {
        config_path: shared.config_path,
        config_explicit: shared.config_explicit,
        tree: shared.tree,
        against_tree: shared.against_tree,
        in_place: shared.in_place,
        no_sandbox: shared.no_sandbox,
        question,
        default_agent_preset,
    })
}

pub(crate) fn ask_help_command() -> Command {
    // Preserve the pre-refactor CLI contract: `canon ask` rejects `--scope`.
    // The temporary expectation's q-scope is owned by interrogation policy.
    command_with_shared_args(
        Command::new("ask")
            .bin_name("canon ask")
            .about("Ask one canon-style question about the project."),
        "Ask against this Git tree [default: :staged]",
        "Ask in the current directory directly",
    )
    .arg(
        value_arg("preset")
            .long("preset")
            .value_name("PRESET")
            .help("Select a preset by name for the question [default: default]"),
    )
    .arg(
        value_arg("question")
            .value_name("QUESTION")
            .help("Question to ask")
            .required(true),
    )
    .after_help(
        "Examples:\n  canon ask \"Does the app expose Undo?\"\n      Ask a one-off question.",
    )
}
