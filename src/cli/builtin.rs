use super::error::CommandError;
use super::help::{
    gate_help_command, hook_help_command, init_help_command, print_help_if_requested,
};
use crate::check::{check_help_command, run_check_command, run_show_command, show_help_command};
use crate::gate::run_gate_command;
use crate::hooks::{run_hook_command, run_init};
use crate::project::{git_project_root, project_root_or_current};
use clap::Command as ClapCommand;
use std::env;
use std::ffi::OsString;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinCommand {
    Init,
    Hook,
    Check,
    Show,
    Gate,
}

impl BuiltinCommand {
    pub(super) fn all() -> &'static [BuiltinCommand] {
        &[
            BuiltinCommand::Init,
            BuiltinCommand::Hook,
            BuiltinCommand::Check,
            BuiltinCommand::Show,
            BuiltinCommand::Gate,
        ]
    }

    pub(super) fn parse(value: &str) -> Option<BuiltinCommand> {
        match value {
            "init" => Some(BuiltinCommand::Init),
            "hook" => Some(BuiltinCommand::Hook),
            "check" => Some(BuiltinCommand::Check),
            "show" => Some(BuiltinCommand::Show),
            "gate" => Some(BuiltinCommand::Gate),
            _ => None,
        }
    }

    pub(super) fn help_command(self) -> ClapCommand {
        match self {
            BuiltinCommand::Init => init_help_command(),
            BuiltinCommand::Hook => hook_help_command(),
            BuiltinCommand::Check => check_help_command(),
            BuiltinCommand::Show => show_help_command(),
            BuiltinCommand::Gate => gate_help_command(),
        }
    }

    pub(super) fn run(self, args: &[OsString]) -> Result<(), CommandError> {
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
                let current_dir = env::current_dir()
                    .map_err(|err| format!("failed to read current dir: {err}"))?;
                let git_root = git_project_root(&current_dir).ok();
                let explicit_in_place = args.iter().any(|arg| arg == "--in-place");
                let default_in_place = git_root.is_none();
                let root = if explicit_in_place || default_in_place {
                    current_dir
                } else {
                    git_root.expect("git_root is present when default_in_place is false")
                };
                run_check_command(&root, args, default_in_place)
            }
            BuiltinCommand::Show => {
                let root = git_project_root(Path::new("."))?;
                run_show_command(&root, args)
            }
            BuiltinCommand::Gate => {
                let root = git_project_root(Path::new("."))?;
                run_gate_command(&root, args)
            }
        }
    }
}
