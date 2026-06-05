use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::process;

use crate::check::{check_help_command, run_check_command};
use crate::gate::run_gate_command;
use crate::hooks::{run_hook_command, run_init};
use crate::logs::DiagnosticLogError;
use crate::notes::{
    append_note, arg_to_string, collect_text_or_stdin, delete_note, ensure_note, read_note,
    require_key, run_rg, write_note,
};
use crate::output::write_stdout_line;
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
    const ALL: [NoteCommand; 7] = [
        NoteCommand::Pwd,
        NoteCommand::Path,
        NoteCommand::Read,
        NoteCommand::Write,
        NoteCommand::Append,
        NoteCommand::Delete,
        NoteCommand::Search,
    ];

    fn parse(value: &str) -> Option<NoteCommand> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| command.aliases().contains(&value))
    }

    fn aliases(self) -> &'static [&'static str] {
        match self {
            NoteCommand::Pwd => &["pwd"],
            NoteCommand::Path => &["p", "path"],
            NoteCommand::Read => &["r", "read"],
            NoteCommand::Write => &["w", "write"],
            NoteCommand::Append => &["a", "append"],
            NoteCommand::Delete => &["d", "del", "delete", "rm"],
            NoteCommand::Search => &["rg", "g"],
        }
    }

    fn help_name(self) -> &'static str {
        match self {
            NoteCommand::Pwd => "pwd",
            NoteCommand::Path => "path",
            NoteCommand::Read => "read",
            NoteCommand::Write => "write",
            NoteCommand::Append => "append",
            NoteCommand::Delete => "delete",
            NoteCommand::Search => "rg",
        }
    }

    fn help_bin_name(self) -> &'static str {
        match self {
            NoteCommand::Pwd => "canon pwd",
            NoteCommand::Path => "canon path",
            NoteCommand::Read => "canon read",
            NoteCommand::Write => "canon write",
            NoteCommand::Append => "canon append",
            NoteCommand::Delete => "canon delete",
            NoteCommand::Search => "canon rg",
        }
    }

    fn help_about(self) -> &'static str {
        match self {
            NoteCommand::Pwd => "Print the canon project root",
            NoteCommand::Path => "Print the path for a canon note",
            NoteCommand::Read => "Read a canon note",
            NoteCommand::Write => "Write a canon note",
            NoteCommand::Append => "Append to a canon note",
            NoteCommand::Delete => "Delete a canon note",
            NoteCommand::Search => "Search canon notes with ripgrep",
        }
    }

    fn help_command(self) -> ClapCommand {
        ClapCommand::new(self.help_name())
            .bin_name(self.help_bin_name())
            .about(self.help_about())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinCommand {
    Init,
    Hook,
    Check,
    Gate,
}

impl BuiltinCommand {
    fn all() -> &'static [BuiltinCommand] {
        &[
            BuiltinCommand::Init,
            BuiltinCommand::Hook,
            BuiltinCommand::Check,
            BuiltinCommand::Gate,
        ]
    }

    fn parse(value: &str) -> Option<BuiltinCommand> {
        match value {
            "init" => Some(BuiltinCommand::Init),
            "hook" => Some(BuiltinCommand::Hook),
            "check" => Some(BuiltinCommand::Check),
            "gate" => Some(BuiltinCommand::Gate),
            _ => None,
        }
    }

    fn help_command(self) -> ClapCommand {
        match self {
            BuiltinCommand::Init => init_help_command(),
            BuiltinCommand::Hook => hook_help_command(),
            BuiltinCommand::Check => check_help_command(),
            BuiltinCommand::Gate => gate_help_command(),
        }
    }

    fn run(self, args: &[OsString]) -> Result<(), CommandError> {
        if print_help_if_requested(args, self.help_command())? {
            return Ok(());
        }
        match self {
            BuiltinCommand::Init => {
                if !args.is_empty() {
                    return Err(CommandError::InitDoesNotAcceptArguments);
                }
                let root = project_root_or_current(Path::new("."))?;
                run_init(&root).map_err(CommandError::from)
            }
            BuiltinCommand::Hook => {
                let root = git_project_root(Path::new("."))?;
                run_hook_command(&root, args).map_err(CommandError::from)
            }
            BuiltinCommand::Check => {
                let root = git_project_root(Path::new("."))?;
                run_check_command(&root, args)
            }
            BuiltinCommand::Gate => {
                let root = git_project_root(Path::new("."))?;
                run_gate_command(&root, args)
            }
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
        let _ = write_command_error_line(&err);
    }
    err
}

fn write_command_error_line(err: &CommandError) -> Result<(), String> {
    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    writeln!(stderr, "Error: {}", err)
        .map_err(|source| format!("failed to write command error to stderr: {}", source))?;
    stderr
        .flush()
        .map_err(|source| format!("failed to flush command error to stderr: {}", source))
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

fn print_help_if_requested(args: &[OsString], command: ClapCommand) -> Result<bool, CommandError> {
    if !command_help_requested(args) {
        return Ok(false);
    }
    print_clap_help(command)?;
    Ok(true)
}

fn print_clap_help(mut command: ClapCommand) -> Result<(), String> {
    let stdout = io::stdout();
    let mut stdout = FlushingTrailingByteWriter::new(stdout.lock());
    command
        .write_help(&mut stdout)
        .map_err(|err| format!("failed to write help to stdout: {}", err))?;
    if stdout.last_byte() != Some(b'\n') {
        stdout
            .write_all(b"\n")
            .map_err(|err| format!("failed to write help newline to stdout: {}", err))?;
    }
    Ok(())
}

struct FlushingTrailingByteWriter<W> {
    inner: W,
    last_byte: Option<u8>,
}

impl<W> FlushingTrailingByteWriter<W> {
    fn new(inner: W) -> FlushingTrailingByteWriter<W> {
        FlushingTrailingByteWriter {
            inner,
            last_byte: None,
        }
    }

    fn last_byte(&self) -> Option<u8> {
        self.last_byte
    }
}

impl<W: Write> Write for FlushingTrailingByteWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        if written > 0 {
            self.last_byte = Some(bytes[written - 1]);
            self.inner.flush()?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn root_help_command() -> ClapCommand {
    let command = ClapCommand::new("canon")
        .about("AI linter for project expectations")
        .subcommands(
            BuiltinCommand::all()
                .iter()
                .copied()
                .map(BuiltinCommand::help_command),
        );
    NoteCommand::ALL
        .into_iter()
        .fold(command, |command, note_command| {
            command.subcommand(note_command.help_command())
        })
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
