mod fs;
mod git;

// This module manages Git pre-commit hook installation only.
use self::fs::make_executable;
use self::fs::HookFile;
use self::git::{
    configure_git_hooks_path, git_hooks_path_matches, unset_git_hooks_path,
    uses_canon_git_hooks_path, HookInstallPreflight,
};
use crate::fs_util::{ensure_dir_without_symlinks, remove_optional_file, replace_file};
use crate::output::{write_stderr, write_stdout, write_stdout_line};
use clap::error::ErrorKind as ClapErrorKind;
use clap::Command as ClapCommand;
use std::ffi::OsString;
use std::path::Path;

// Canon installs the hook at Git's default repository hook path and reports
// that same path through the public command output.
pub(super) const DEFAULT_GIT_HOOKS_PATH: &str = ".git/hooks";
pub(super) const DEFAULT_GIT_PRE_COMMIT_HOOK_PATH: &str = ".git/hooks/pre-commit";
pub(super) const DEFAULT_PRE_COMMIT_GIT_PATH: &str = "hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../../resources/git-hooks/pre-commit");
const PRE_COMMIT_HOOK_MANUAL_ADVICE: &str =
    "Can't safely install pre-commit hook.\n▷ Add `canon gate` manually to the existing pre-commit setup or ask a human to handle it.";
const PRE_COMMIT_HOOK_UNINSTALL_MANUAL_ADVICE: &str =
    "Can't safely uninstall pre-commit hook because it is not Canon's hook.\n▷ Remove it manually only if intended, or ask a human to handle it.";
pub(super) const GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL: &str =
    "Can't safely install pre-commit hook: canon pre-commit install requires a Git worktree.";

pub(crate) fn pre_commit_help_command() -> ClapCommand {
    ClapCommand::new("pre-commit")
        .bin_name("canon pre-commit")
        .about("Manage the canon pre-commit hook")
        .subcommand_required(true)
        .subcommand(ClapCommand::new("install").about("Install the canon pre-commit hook"))
        .subcommand(ClapCommand::new("uninstall").about("Uninstall the canon pre-commit hook"))
}

pub(crate) fn run_pre_commit_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    let Some(action) = parse_pre_commit_action(args)? else {
        return Ok(());
    };
    match action {
        HookAction::Install => run_hook_install(root),
        HookAction::Uninstall => run_hook_uninstall(root),
    }
}

enum HookAction {
    Install,
    Uninstall,
}

fn parse_pre_commit_action(args: &[OsString]) -> Result<Option<HookAction>, String> {
    let argv = std::iter::once(OsString::from("canon pre-commit"))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    let matches = match pre_commit_help_command().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(err)
            if matches!(
                err.kind(),
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion
            ) =>
        {
            write_clap_display_error(&err)?;
            return Ok(None);
        }
        Err(err) => return Err(err.to_string()),
    };
    match matches.subcommand_name() {
        Some("install") => Ok(Some(HookAction::Install)),
        Some("uninstall") => Ok(Some(HookAction::Uninstall)),
        Some(action) => Err(format!("unknown pre-commit command: {}", action)),
        None => unreachable!("clap requires a pre-commit subcommand"),
    }
}

fn write_clap_display_error(err: &clap::Error) -> Result<(), String> {
    let rendered = err.to_string();
    if err.use_stderr() {
        write_stderr(&rendered)
    } else {
        write_stdout(&rendered)
    }
}

pub(crate) fn run_hook_install(root: &Path) -> Result<(), String> {
    let preflight = HookInstallPreflight::load(root)?;
    preflight_git_worktree(&preflight)?;
    preflight_pre_commit_hook_ownership(&preflight, HookAction::Install)?;
    preflight_git_hooks_path(&preflight)?;
    install_pre_commit_hook(root, &preflight)
}

fn preflight_git_worktree(preflight: &HookInstallPreflight) -> Result<(), String> {
    if preflight.is_git_worktree {
        return Ok(());
    }
    Err(GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL.to_string())
}

fn preflight_pre_commit_hook_ownership(
    preflight: &HookInstallPreflight,
    action: HookAction,
) -> Result<(), String> {
    // The default Git hook path is user-owned whenever Git is not already
    // Canon's exact hook. A matching `core.hooksPath` identifies the directory,
    // not ownership of a file already present in that directory.
    match &preflight.default_pre_commit_hook {
        HookFile::Missing => Ok(()),
        HookFile::Regular(contents) if contents == DEFAULT_PRE_COMMIT_HOOK => Ok(()),
        HookFile::Regular(_) | HookFile::Unverifiable => match action {
            HookAction::Install => Err(pre_commit_hook_manual_advice()),
            HookAction::Uninstall => Err(PRE_COMMIT_HOOK_UNINSTALL_MANUAL_ADVICE.to_string()),
        },
    }
}

fn preflight_git_hooks_path(preflight: &HookInstallPreflight) -> Result<(), String> {
    // [Y8] Canon owns the hook directory only when effective Git configuration
    // has no custom hook manager or already points at Canon's hook directory.
    // `git config` precedence includes system, global, worktree, and local
    // sources; any effective non-Canon value needs manual handling.
    if let Some(existing) = &preflight.current_git_hooks_path {
        if !git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing) {
            return Err(pre_commit_hook_manual_advice());
        }
    }
    Ok(())
}

fn install_pre_commit_hook(root: &Path, preflight: &HookInstallPreflight) -> Result<(), String> {
    let hook_path = preflight.pre_commit_hook_path.as_path();
    if let Some(parent) = hook_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    replace_file(hook_path, DEFAULT_PRE_COMMIT_HOOK)?;
    make_executable(hook_path)?;
    configure_git_hooks_path(root, preflight)?;
    write_stdout_line(&format!("Installed {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

pub(crate) fn run_hook_uninstall(root: &Path) -> Result<(), String> {
    let preflight = HookInstallPreflight::load(root)?;
    // [Y8] Verify file ownership before changing either Git configuration or
    // the hook path. A foreign hook must leave both states untouched.
    preflight_pre_commit_hook_ownership(&preflight, HookAction::Uninstall)?;
    let uses_canon_hooks_path = uses_canon_git_hooks_path(&preflight);
    if uses_canon_hooks_path {
        unset_git_hooks_path(root)?;
    }
    remove_optional_file(&preflight.pre_commit_hook_path)?;
    // The hook file has been removed by this point, and there is no later
    // uninstall work to delay the flushed success line.
    write_stdout_line(&format!("Uninstalled {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

fn pre_commit_hook_manual_advice() -> String {
    PRE_COMMIT_HOOK_MANUAL_ADVICE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn preflight(
        default_pre_commit_hook: HookFile,
        current_git_hooks_path: Option<&str>,
    ) -> HookInstallPreflight {
        let root = PathBuf::from("repo");
        HookInstallPreflight {
            current_git_hooks_path: current_git_hooks_path.map(str::to_string),
            default_pre_commit_hook,
            is_git_worktree: true,
            git_hooks_path: root.join(DEFAULT_GIT_HOOKS_PATH),
            pre_commit_hook_path: root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH),
            root,
        }
    }

    #[test] // xpec: Y8
    fn foreign_pre_commit_hook_requires_documented_manual_setup() {
        let preflight = preflight(HookFile::Unverifiable, None);

        let error =
            preflight_pre_commit_hook_ownership(&preflight, HookAction::Install).unwrap_err();

        assert_eq!(error, PRE_COMMIT_HOOK_MANUAL_ADVICE);
    }

    #[test] // xpec: Y8
    fn external_hook_manager_requires_documented_manual_setup() {
        let preflight = preflight(HookFile::Missing, Some("global-hooks"));

        let error = preflight_git_hooks_path(&preflight).unwrap_err();

        assert_eq!(error, PRE_COMMIT_HOOK_MANUAL_ADVICE);
    }

    #[test] // xpec: Y8
    fn install_requires_a_git_worktree() {
        let mut preflight = preflight(HookFile::Missing, None);
        preflight.is_git_worktree = false;

        let error = preflight_git_worktree(&preflight).unwrap_err();

        assert_eq!(error, GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL);
    }
}
