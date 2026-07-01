use super::error::CommandError;
use super::help::{
    gate_help_command, init_help_command, pre_commit_help_command, print_help_if_requested,
};
use crate::check::{
    ask_help_command, check_help_command, run_ask_command, run_check_command, run_show_command,
    show_help_command,
};
use crate::gate::run_gate_command;
use crate::hooks::run_pre_commit_command;
use crate::init::run_init;
use crate::project::{git_project_root, project_root_or_current};
use clap::Command as ClapCommand;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinCommand {
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
        if self != BuiltinCommand::PreCommit && print_help_if_requested(args, self.help_command())?
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
            BuiltinCommand::Ask => {
                let (root, default_in_place) = check_like_root(args)?;
                run_ask_command(&root, args, default_in_place)
            }
            BuiltinCommand::Check => {
                let (root, default_in_place) = check_like_root(args)?;
                run_check_command(&root, args, default_in_place)
            }
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

fn check_like_root(args: &[OsString]) -> Result<(std::path::PathBuf, bool), CommandError> {
    let current_dir =
        env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?;
    let git_root = git_project_root(&current_dir).ok();
    let explicit_in_place = args_include_option_before_separator(args, "--in-place");
    let default_in_place = git_root.is_none();
    // This is only the in-place root-selection rule. The rest of the in-place
    // contract is split across command parsing (`src/check/command/args.rs`),
    // in-place expectation validation and orchestration
    // (`src/check/command/execution/in_place.rs` and `run.rs`), runtime
    // scope/session behavior (`src/check/interrogation/state.rs`), and config
    // expansion (`src/repo_inspection/mod.rs`).
    let root = if explicit_in_place || default_in_place {
        current_dir
    } else {
        git_root.expect("git_root is present when default_in_place is false")
    };
    Ok((root, default_in_place))
}

fn args_include_option_before_separator(args: &[OsString], option: &str) -> bool {
    args.iter()
        .take_while(|arg| arg.as_os_str() != OsStr::new("--"))
        .any(|arg| arg == option)
}

#[cfg(test)]
mod tests {
    use super::args_include_option_before_separator;
    use std::ffi::OsString;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn option_scan_stops_at_separator() {
        assert!(!args_include_option_before_separator(
            &os_args(&["--", "--in-place"]),
            "--in-place"
        ));
    }

    #[test]
    fn option_scan_finds_option_before_separator() {
        assert!(args_include_option_before_separator(
            &os_args(&["--in-place", "--", "--ignored"]),
            "--in-place"
        ));
    }
}
