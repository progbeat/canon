use crate::check::cli_args::args_request_in_place;
use crate::cli::CommandError;
use crate::git::git_project_root;
use crate::logs::DiagnosticLogPlan;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

pub(super) struct CheckLikeRoot {
    pub(super) root: PathBuf,
    pub(super) default_in_place: bool,
    in_place: bool,
}

pub(super) fn resolve_check_like_root(args: &[OsString]) -> Result<CheckLikeRoot, CommandError> {
    let current_dir =
        env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?;
    let explicit_in_place = args_request_in_place(args);
    // [90] An explicit in-place invocation selects the checked directory
    // without Git-backed check-root discovery.
    if explicit_in_place {
        return Ok(CheckLikeRoot {
            root: current_dir,
            default_in_place: false,
            in_place: true,
        });
    }
    let git_root = git_project_root(&current_dir).ok();
    let default_in_place = git_root.is_none();
    // This is only the in-place root-selection rule. The rest of the in-place
    // contract is split across command parsing, config validation, command
    // orchestration, runtime scope/session behavior, and config expansion.
    let root = if default_in_place {
        current_dir
    } else {
        git_root.expect("git_root is present when default_in_place is false")
    };
    Ok(CheckLikeRoot {
        root,
        default_in_place,
        in_place: default_in_place,
    })
}

pub(super) fn git_backed_diagnostic_log_plan(
    command_root: &CheckLikeRoot,
) -> Option<DiagnosticLogPlan> {
    if command_root.in_place {
        // [90] This is an explicit command-mode boundary, not an unset
        // Git-backed configuration value. Reading `canon.logs.maxSize` here
        // would violate the in-place requirement to ignore all Git information.
        None
    } else {
        Some(DiagnosticLogPlan::prepare(&command_root.root))
    }
}
