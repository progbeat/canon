use super::shared::{
    command_with_shared_args, no_sandbox_arg, parse_command_matches, parse_shared_command_args,
    validate_in_place_options,
};
use crate::check::core::CheckCommandArgs;
use crate::check::engine::selection::{add_check_option_args, raw_check_options_from_matches};
use crate::check::CHECK_PATH;
use crate::git::{DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use clap::Command;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn parse_check_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<CheckCommandArgs, String> {
    let matches = parse_command_matches(check_help_command(), args)?;
    let no_sandbox = matches.get_flag("no_sandbox");
    let shared = parse_shared_command_args(&matches, default_in_place, no_sandbox)?;
    let options = raw_check_options_from_matches(&matches)?;
    if shared.in_place {
        // This parser rejects CLI options whose meaning depends on a Git tree
        // or path-hiding behavior. Config-level mode compatibility runs after
        // raw config expansion, so generators/includes have already resolved.
        validate_in_place_options(
            "canon check",
            shared.tree_explicit,
            shared.against_tree_explicit,
        )?;
    }

    Ok(CheckCommandArgs {
        sources_have_command_default_values: shared.config_path == Path::new(CHECK_PATH)
            && shared.tree == STAGED_TREE_ARG
            && shared.against_tree == DEFAULT_AGAINST_TREE_ARG,
        config_path: shared.config_path,
        tree: shared.tree,
        against_tree: shared.against_tree,
        in_place: shared.in_place,
        no_sandbox: shared.no_sandbox,
        options,
    })
}

pub(crate) fn check_help_command() -> Command {
    let command = command_with_shared_args(
        Command::new("check")
            .bin_name("canon check")
            .about("Check whether project files meet human expectations written in the canon."),
        "Check this Git tree [default: :staged]",
        "Check the current directory directly",
    )
    .arg(no_sandbox_arg());
    // `add_check_option_args` supplies the documented public `--keep-going`
    // option and the selector argument used by the examples below, including
    // `not:<ID-PREFIX>` exclusions. It also registers hidden internal controls
    // that are intentionally absent from `canon check --help`.
    add_check_option_args(command).after_help(
            "Examples:\n  canon check\n      Check staged content against all canon expectations.\n\n  canon check a7F K9m\n      Check canon expectations selected by ID prefix.\n\n  canon check not:a7F not:K9m\n      Check all expectations except those whose IDs start with a7F or K9m.\n\n  canon check --tree HEAD --against-tree HEAD~1 a7F\n      Check one canon expectation on HEAD with comparison against the previous commit.",
        )
}
