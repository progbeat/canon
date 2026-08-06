use super::error::CommandError;
use super::help::{
    gate_help_command, init_help_command, pre_commit_help_command, print_help_if_requested,
};
use crate::check::{
    ask_help_command, check_help_command, run_ask_command, run_check_command, run_show_command,
    show_help_command,
};
use crate::gate::run_gate_command;
use crate::git::git_project_root;
use crate::hooks::run_pre_commit_command;
use crate::init::run_init;
use crate::project::project_root_or_current;
use clap::Command as ClapCommand;
use std::ffi::OsString;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinCommand {
    // Keep this enum aligned with the public command surface. `ask` and
    // `pre-commit` are public builtins; the old `hook` command is not.
    // `Ask` and `Check` dispatch to distinct command boundaries because their
    // public response-output contracts differ.
    Init,
    PreCommit,
    Ask,
    Check,
    Show,
    Gate,
}

impl BuiltinCommand {
    pub(super) fn all() -> &'static [BuiltinCommand] {
        &[
            BuiltinCommand::Init,
            BuiltinCommand::PreCommit,
            BuiltinCommand::Ask,
            BuiltinCommand::Check,
            BuiltinCommand::Show,
            BuiltinCommand::Gate,
        ]
    }

    pub(super) fn parse(value: &str) -> Option<BuiltinCommand> {
        match value {
            "init" => Some(BuiltinCommand::Init),
            "pre-commit" => Some(BuiltinCommand::PreCommit),
            "ask" => Some(BuiltinCommand::Ask),
            "check" => Some(BuiltinCommand::Check),
            "show" => Some(BuiltinCommand::Show),
            "gate" => Some(BuiltinCommand::Gate),
            _ => None,
        }
    }

    pub(super) fn help_command(self) -> ClapCommand {
        match self {
            BuiltinCommand::Init => init_help_command(),
            BuiltinCommand::PreCommit => pre_commit_help_command(),
            BuiltinCommand::Ask => ask_help_command(),
            BuiltinCommand::Check => check_help_command(),
            BuiltinCommand::Show => show_help_command(),
            BuiltinCommand::Gate => gate_help_command(),
        }
    }

    pub(super) fn run(self, args: &[OsString]) -> Result<(), CommandError> {
        if !matches!(
            self,
            BuiltinCommand::PreCommit | BuiltinCommand::Ask | BuiltinCommand::Check
        ) && print_help_if_requested(args, self.help_command())?
        {
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
            BuiltinCommand::PreCommit => {
                if args
                    .iter()
                    .any(|arg| arg == std::ffi::OsStr::new("-h") || arg == "--help")
                {
                    return run_pre_commit_command(Path::new("."), args)
                        .map_err(CommandError::from);
                }
                let root = git_project_root(Path::new("."))?;
                run_pre_commit_command(&root, args).map_err(CommandError::from)
            }
            BuiltinCommand::Ask => run_ask_command(args),
            BuiltinCommand::Check => run_check_command(args),
            BuiltinCommand::Show => {
                let root = git_project_root(Path::new("."))?;
                run_show_command(&root, args)
            }
            BuiltinCommand::Gate => {
                let root = git_project_root(Path::new(".")).map_err(|err| {
                    format!("{err}\n▷ Run `canon gate` from inside a Git worktree.")
                })?;
                run_gate_command(&root, args)
            }
        }
    }
}
