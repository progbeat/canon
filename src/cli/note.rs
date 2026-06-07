use super::error::CommandError;
use crate::notes::{
    append_note, collect_text_or_stdin, delete_note, ensure_note, read_note, require_key, run_rg,
    write_note,
};
use crate::output::write_stdout_line;
use crate::project::print_root;
use crate::project_types::Config;
use clap::Command as ClapCommand;
use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoteCommand {
    Pwd,
    Path,
    Read,
    Write,
    Append,
    Delete,
    Search,
}

impl NoteCommand {
    pub(super) const ALL: [NoteCommand; 7] = [
        NoteCommand::Pwd,
        NoteCommand::Path,
        NoteCommand::Read,
        NoteCommand::Write,
        NoteCommand::Append,
        NoteCommand::Delete,
        NoteCommand::Search,
    ];

    pub(super) fn parse(value: &str) -> Option<NoteCommand> {
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

    pub(super) fn help_command(self) -> ClapCommand {
        ClapCommand::new(self.help_name())
            .bin_name(self.help_bin_name())
            .about(self.help_about())
    }
}

pub(super) fn run_note_command(
    command: NoteCommand,
    config: &Config,
    args: &[OsString],
) -> Result<(), CommandError> {
    match command {
        NoteCommand::Pwd => {
            if args.len() != 1 {
                return Err(CommandError::PwdDoesNotAcceptArguments);
            }
            print_root(config)?;
        }
        NoteCommand::Path => {
            let key = require_key(args, 1)?;
            let note = ensure_note(config, key)?;
            write_stdout_line(&note.path.display().to_string())?;
        }
        NoteCommand::Read => {
            let key = require_key(args, 1)?;
            read_note(config, key)?;
        }
        NoteCommand::Write => {
            let key = require_key(args, 1)?;
            let text = collect_text_or_stdin(args, 2)?;
            write_note(config, key, &text)?;
        }
        NoteCommand::Append => {
            let key = require_key(args, 1)?;
            let text = collect_text_or_stdin(args, 2)?;
            append_note(config, key, &text)?;
        }
        NoteCommand::Delete => {
            let key = require_key(args, 1)?;
            delete_note(config, key)?;
        }
        NoteCommand::Search => {
            run_rg(config, &args[1..])?;
        }
    }
    Ok(())
}
