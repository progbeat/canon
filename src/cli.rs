use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process;

use crate::check_command::run_check_command;
use crate::evaluator::print_help;
use crate::gate::run_gate_command;
use crate::hooks::{run_hook_command, run_init};
use crate::logging::DiagnosticLogError;
use crate::notes::{append_note, delete_note, ensure_note, read_note, write_note};
use crate::notes_cli::{arg_to_string, collect_text_or_stdin, require_key, run_rg};
use crate::output::{write_stderr_line, write_stdout_line};
use crate::project::{git_project_root, print_root, project_root_or_current};
use crate::project_types::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandError {
    Message(Cow<'static, str>),
    InitDoesNotAcceptArguments,
    PwdDoesNotAcceptArguments,
    UnknownOption(String),
    UnknownCommand(String),
    CheckFailed,
    GateFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteCommand {
    Pwd,
    Path,
    Read,
    Write,
    Append,
    Delete,
    Search,
}

impl NoteCommand {
    fn parse(value: &str) -> Option<NoteCommand> {
        match value {
            "pwd" => Some(NoteCommand::Pwd),
            "p" | "path" => Some(NoteCommand::Path),
            "r" | "read" => Some(NoteCommand::Read),
            "w" | "write" => Some(NoteCommand::Write),
            "a" | "append" => Some(NoteCommand::Append),
            "d" | "del" | "delete" | "rm" => Some(NoteCommand::Delete),
            "rg" | "g" => Some(NoteCommand::Search),
            _ => None,
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> CommandError {
        CommandError::Message(Cow::Owned(message))
    }
}

impl From<DiagnosticLogError> for CommandError {
    fn from(err: DiagnosticLogError) -> CommandError {
        CommandError::Message(Cow::Owned(err.to_string()))
    }
}

impl From<&'static str> for CommandError {
    fn from(message: &'static str) -> CommandError {
        CommandError::Message(Cow::Borrowed(message))
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Message(message) => formatter.write_str(message),
            CommandError::InitDoesNotAcceptArguments => {
                formatter.write_str("init does not accept arguments")
            }
            CommandError::PwdDoesNotAcceptArguments => {
                formatter.write_str("pwd does not accept arguments")
            }
            CommandError::UnknownOption(option) => write!(formatter, "unknown option: {option}"),
            CommandError::UnknownCommand(command) => {
                write!(formatter, "unknown command: {command}")
            }
            CommandError::CheckFailed => formatter.write_str("canon check failed"),
            CommandError::GateFailed => formatter.write_str("canon gate failed"),
        }
    }
}

pub(crate) fn main() {
    if run(env::args_os().skip(1).collect()).is_err() {
        process::exit(1);
    }
}

pub(crate) fn command_error_has_public_diagnostic(err: &CommandError) -> bool {
    // These commands already wrote their public diagnostics before returning a
    // sentinel error for the process exit status.
    !matches!(err, CommandError::CheckFailed | CommandError::GateFailed)
}

pub(crate) fn run(args: Vec<OsString>) -> Result<(), CommandError> {
    run_command(args).map_err(report_command_error)
}

fn report_command_error(err: CommandError) -> CommandError {
    if command_error_has_public_diagnostic(&err) {
        let _ = write_stderr_line(&format!("Error: {}", err));
    }
    err
}

fn run_command(args: Vec<OsString>) -> Result<(), CommandError> {
    if args.is_empty() {
        let config = Config::from_env()?;
        print_root(&config)?;
        return Ok(());
    }

    let first = arg_to_string(&args[0])?;
    let note_command = match first.as_str() {
        "init" => {
            if args.len() != 1 {
                return Err(CommandError::InitDoesNotAcceptArguments);
            }
            let root = project_root_or_current(Path::new("."))?;
            return run_init(&root).map_err(CommandError::from);
        }
        "hook" => {
            let root = git_project_root(Path::new("."))?;
            return run_hook_command(&root, &args[1..]).map_err(CommandError::from);
        }
        "check" => {
            let root = git_project_root(Path::new("."))?;
            return run_check_command(&root, &args[1..]);
        }
        "gate" => {
            let root = git_project_root(Path::new("."))?;
            return run_gate_command(&root, &args[1..]);
        }
        "-h" | "--help" | "help" => {
            print_help()?;
            return Ok(());
        }
        value => {
            if let Some(command) = NoteCommand::parse(value) {
                command
            } else if first.starts_with('-') {
                return Err(CommandError::UnknownOption(first));
            } else {
                return Err(CommandError::UnknownCommand(first));
            }
        }
    };

    let config = Config::from_env()?;
    match note_command {
        NoteCommand::Pwd => {
            if args.len() != 1 {
                return Err(CommandError::PwdDoesNotAcceptArguments);
            }
            print_root(&config)?;
        }
        NoteCommand::Path => {
            let key = require_key(&args, 1)?;
            let note = ensure_note(&config, key)?;
            write_stdout_line(&note.path.display().to_string())?;
        }
        NoteCommand::Read => {
            let key = require_key(&args, 1)?;
            read_note(&config, key)?;
        }
        NoteCommand::Write => {
            let key = require_key(&args, 1)?;
            let text = collect_text_or_stdin(&args, 2)?;
            write_note(&config, key, &text)?;
        }
        NoteCommand::Append => {
            let key = require_key(&args, 1)?;
            let text = collect_text_or_stdin(&args, 2)?;
            append_note(&config, key, &text)?;
        }
        NoteCommand::Delete => {
            let key = require_key(&args, 1)?;
            delete_note(&config, key)?;
        }
        NoteCommand::Search => {
            run_rg(&config, &args[1..])?;
        }
    }

    Ok(())
}
