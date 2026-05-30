use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process;

use crate::check::command::run_check_command;
use crate::check::command_args::{check_help_command, check_help_requested};
use crate::gate::run_gate_command;
use crate::hooks::{run_hook_command, run_init};
use crate::logs::DiagnosticLogError;
use crate::notes::cli::{arg_to_string, collect_text_or_stdin, require_key, run_rg};
use crate::notes::{append_note, delete_note, ensure_note, read_note, write_note};
use crate::output::{write_stderr_line, write_stdout, write_stdout_line};
use crate::project::{git_project_root, print_root, project_root_or_current};
use crate::project_types::Config;
use clap::Command as ClapCommand;

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
            if command_help_requested(&args[1..]) {
                print_clap_help(init_help_command())?;
                return Ok(());
            }
            if args.len() != 1 {
                return Err(CommandError::InitDoesNotAcceptArguments);
            }
            let root = project_root_or_current(Path::new("."))?;
            return run_init(&root).map_err(CommandError::from);
        }
        "hook" => {
            if command_help_requested(&args[1..]) {
                print_clap_help(hook_help_command())?;
                return Ok(());
            }
            let root = git_project_root(Path::new("."))?;
            return run_hook_command(&root, &args[1..]).map_err(CommandError::from);
        }
        "check" => {
            if check_help_requested(&args[1..]) {
                print_clap_help(check_help_command())?;
                return Ok(());
            }
            let root = git_project_root(Path::new("."))?;
            return run_check_command(&root, &args[1..]);
        }
        "gate" => {
            if command_help_requested(&args[1..]) {
                print_clap_help(gate_help_command())?;
                return Ok(());
            }
            let root = git_project_root(Path::new("."))?;
            return run_gate_command(&root, &args[1..]);
        }
        "-h" | "--help" | "help" => {
            print_clap_help(root_help_command())?;
            return Ok(());
        }
        value => {
            if let Some(command) = NoteCommand::parse(value) {
                if command_help_requested(&args[1..]) {
                    print_clap_help(note_help_command(value))?;
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

fn command_help_requested(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| arg == std::ffi::OsStr::new("-h") || arg == std::ffi::OsStr::new("--help"))
}

fn print_clap_help(mut command: ClapCommand) -> Result<(), String> {
    let mut help = command.render_help().to_string();
    if !help.ends_with('\n') {
        help.push('\n');
    }
    write_stdout(&help)
}

fn root_help_command() -> ClapCommand {
    ClapCommand::new("canon")
        .about("AI linter for project expectations")
        .subcommand(init_help_command())
        .subcommand(hook_help_command())
        .subcommand(check_help_command())
        .subcommand(gate_help_command())
        .subcommand(note_help_command("pwd"))
        .subcommand(note_help_command("path"))
        .subcommand(note_help_command("read"))
        .subcommand(note_help_command("write"))
        .subcommand(note_help_command("append"))
        .subcommand(note_help_command("delete"))
        .subcommand(note_help_command("rg"))
}

fn init_help_command() -> ClapCommand {
    ClapCommand::new("init")
        .bin_name("canon init")
        .about("Create the default canon configuration")
}

fn hook_help_command() -> ClapCommand {
    ClapCommand::new("hook")
        .bin_name("canon hook")
        .about("Manage the canon Git hook")
        .subcommand(ClapCommand::new("install").about("Install the canon Git hook"))
        .subcommand(ClapCommand::new("uninstall").about("Uninstall the canon Git hook"))
}

fn gate_help_command() -> ClapCommand {
    ClapCommand::new("gate")
        .bin_name("canon gate")
        .about("Fail when staged canon expectations regress")
}

fn note_help_command(name: &str) -> ClapCommand {
    match name {
        "p" | "path" => ClapCommand::new("path")
            .bin_name("canon path")
            .about("Print the path for a canon note"),
        "r" | "read" => ClapCommand::new("read")
            .bin_name("canon read")
            .about("Read a canon note"),
        "w" | "write" => ClapCommand::new("write")
            .bin_name("canon write")
            .about("Write a canon note"),
        "a" | "append" => ClapCommand::new("append")
            .bin_name("canon append")
            .about("Append to a canon note"),
        "d" | "del" | "delete" | "rm" => ClapCommand::new("delete")
            .bin_name("canon delete")
            .about("Delete a canon note"),
        "rg" | "g" => ClapCommand::new("rg")
            .bin_name("canon rg")
            .about("Search canon notes with ripgrep"),
        _ => ClapCommand::new("pwd")
            .bin_name("canon pwd")
            .about("Print the canon project root"),
    }
}
