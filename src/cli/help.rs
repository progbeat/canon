use super::builtin::BuiltinCommand;
use super::error::CommandError;
use super::note::NoteCommand;
use clap::Command as ClapCommand;
use std::ffi::OsString;
use std::io::{self, Write};

pub(super) fn print_help_if_requested(
    args: &[OsString],
    command: ClapCommand,
) -> Result<bool, CommandError> {
    if !command_help_requested(args) {
        return Ok(false);
    }
    print_clap_help(command)?;
    Ok(true)
}

fn command_help_requested(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| arg == std::ffi::OsStr::new("-h") || arg == std::ffi::OsStr::new("--help"))
}

pub(super) fn print_clap_help(mut command: ClapCommand) -> Result<(), String> {
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

pub(super) fn root_help_command() -> ClapCommand {
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

pub(super) fn init_help_command() -> ClapCommand {
    ClapCommand::new("init")
        .bin_name("canon init")
        .about("Create the default canon configuration")
}

pub(super) fn pre_commit_help_command() -> ClapCommand {
    crate::hooks::pre_commit_help_command()
}

pub(super) fn gate_help_command() -> ClapCommand {
    ClapCommand::new("gate")
        .bin_name("canon gate")
        .about("Fail when staged canon expectations regress")
}
