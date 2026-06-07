mod builtin;
mod error;
mod help;
mod note;

use builtin::BuiltinCommand;
use error::report_command_error;
use help::{print_clap_help, print_help_if_requested, root_help_command};
use note::{run_note_command, NoteCommand};
use std::env;
use std::ffi::OsString;
use std::process;

use crate::notes::arg_to_string;
use crate::project::print_root;
use crate::project_types::Config;

pub(crate) use error::CommandError;

pub(crate) fn main() {
    if run(env::args_os().skip(1).collect()).is_err() {
        process::exit(1);
    }
}

pub(crate) fn run(args: Vec<OsString>) -> Result<(), CommandError> {
    run_command(args).map_err(report_command_error)
}

fn run_command(args: Vec<OsString>) -> Result<(), CommandError> {
    if args.is_empty() {
        let config = Config::from_env()?;
        print_root(&config)?;
        return Ok(());
    }

    let first = arg_to_string(&args[0])?;
    if let Some(command) = BuiltinCommand::parse(first.as_str()) {
        return command.run(&args[1..]);
    }
    let note_command = match first.as_str() {
        "-h" | "--help" | "help" => {
            print_clap_help(root_help_command())?;
            return Ok(());
        }
        value => {
            if let Some(command) = NoteCommand::parse(value) {
                if print_help_if_requested(&args[1..], command.help_command())? {
                    return Ok(());
                }
                command
            } else if first.starts_with('-') {
                return Err(CommandError::UnknownOption(first));
            } else {
                return Err(CommandError::UnknownCommand(first));
            }
        }
    };

    let config = Config::from_env()?;
    run_note_command(note_command, &config, &args)
}
