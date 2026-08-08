mod ask;
mod evaluation;
mod lifecycle;
mod root;

use crate::check::command::args::check_help_command;
use crate::cli::{print_help_if_requested, CommandError};
pub(crate) use ask::run_ask_command;
use std::ffi::OsString;

pub(crate) fn run_check_command(args: &[OsString]) -> Result<(), CommandError> {
    if print_help_if_requested(args, check_help_command())? {
        return Ok(());
    }
    lifecycle::run_check_command(args)
}
