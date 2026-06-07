mod fs;
mod git;

use self::fs::{
    ensure_project_dir_without_symlinks, make_executable, path_exists_no_follow,
    remove_optional_file, replace_file, write_new_file,
};
use self::git::{
    configure_git_hooks_path, git_hooks_path_matches, has_canon_git_hooks_path,
    unset_git_hooks_path, uses_canon_git_hooks_path, HookInstallPreflight,
};
use crate::check::CHECK_PATH;
use crate::fs_util::ensure_dir_without_symlinks;
use crate::notes::arg_to_string;
use crate::output::write_stdout_line;
use std::ffi::OsString;
use std::path::Path;

// User-facing output names the repository's pre-commit hook role at Git's
// default hook path. The managed script is stored under Canon's git-path state
// directory, and `core.hooksPath` points Git at that resolved hook directory.
pub(super) const DEFAULT_GIT_PRE_COMMIT_HOOK_PATH: &str = ".git/hooks/pre-commit";
pub(super) const DEFAULT_PRE_COMMIT_GIT_PATH: &str = "hooks/pre-commit";
// `${CANON_STATE_DIR}/hooks`, resolved through `git rev-parse --git-path`.
pub(super) const GIT_HOOKS_PATH: &str = "canon/hooks";
// `${CANON_STATE_DIR}/hooks/pre-commit`, resolved through `git rev-parse --git-path`.
pub(super) const PRE_COMMIT_HOOK_PATH: &str = "canon/hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../../resources/git-hooks/pre-commit");
const PRE_COMMIT_HOOK_MANUAL_ADVICE: &str =
    "Can't safely install pre-commit hook.\n▷ Add `canon gate` manually to the existing hook setup or ask a human to handle it.";
pub(super) const GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL: &str =
    "Can't safely install pre-commit hook: canon hook install requires a Git worktree.";

// The canon init seed is compiled into the binary from the default check-config
// template file, not loaded at runtime as an evaluator prompt/instruction.
// Interrogation texts live under resources/prompts.
const DEFAULT_CHECK_CONFIG_TEMPLATE_FILE_CONTENTS: &str =
    include_str!("../../.canon/templates/default/check.yml");

pub(crate) fn run_init(root: &Path) -> Result<(), String> {
    let check_path = root.join(CHECK_PATH);
    if path_exists_no_follow(&check_path)? {
        return Err(format!("{} already exists", CHECK_PATH));
    }

    // These are user-owned project configuration files, not canon runtime
    // state: they live in the worktree so humans can review and version them.
    if let Some(parent) = check_path.parent() {
        ensure_project_dir_without_symlinks(root, parent)?;
    }
    write_new_file(&check_path, DEFAULT_CHECK_CONFIG_TEMPLATE_FILE_CONTENTS)?;
    // This success line becomes eligible only after the config file exists;
    // `write_stdout_line` flushes it immediately and no later init work remains.
    write_stdout_line(&format!("Created {}", CHECK_PATH))?;
    Ok(())
}

pub(crate) fn run_hook_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: canon hook <install|uninstall>".to_string());
    }
    let action = arg_to_string(&args[0])?;
    match action.as_str() {
        "install" => run_hook_install(root),
        "uninstall" => run_hook_uninstall(root),
        _ => Err(format!("unknown hook command: {}", action)),
    }
}

pub(crate) fn run_hook_install(root: &Path) -> Result<(), String> {
    let preflight = HookInstallPreflight::load(root)?;
    preflight_git_worktree(&preflight)?;
    preflight_default_git_pre_commit_hook(&preflight)?;
    preflight_git_hooks_path(&preflight)?;
    install_pre_commit_hook(root, &preflight)
}

fn preflight_git_worktree(preflight: &HookInstallPreflight) -> Result<(), String> {
    if preflight.is_git_worktree {
        return Ok(());
    }
    Err(GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL.to_string())
}

fn preflight_default_git_pre_commit_hook(preflight: &HookInstallPreflight) -> Result<(), String> {
    if uses_canon_git_hooks_path(preflight) {
        return Ok(());
    }
    // Canon-managed reusable hooks live under `PRE_COMMIT_HOOK_PATH` with
    // `core.hooksPath` pointing at `GIT_HOOKS_PATH`. The default Git hook path
    // is user-owned whenever Git is not already configured for Canon, even if a
    // file there happens to look compatible.
    if preflight.default_pre_commit_hook.is_some() {
        return Err(pre_commit_hook_manual_advice());
    }
    Ok(())
}

fn preflight_git_hooks_path(preflight: &HookInstallPreflight) -> Result<(), String> {
    // Canon owns the hook directory only when Git has no custom hook manager or
    // already points at Canon's hook directory. Any other `core.hooksPath`
    // belongs to existing project Git integration and needs manual handling.
    for existing in &preflight.current_git_hooks_paths {
        if !git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing) {
            return Err(pre_commit_hook_manual_advice());
        }
    }
    Ok(())
}

fn install_pre_commit_hook(root: &Path, preflight: &HookInstallPreflight) -> Result<(), String> {
    // The hook script is canon-owned persistent state, so it lives under the
    // repository's git-path state area. The local `core.hooksPath` value is Git
    // configuration: it points Git at that hook directory.
    let hook_path = preflight.pre_commit_hook_path.as_path();
    if let Some(parent) = hook_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    replace_file(hook_path, DEFAULT_PRE_COMMIT_HOOK)?;
    // The managed hook file exists now. Emit that user-visible fact before the
    // remaining install steps so the line is not held until command completion.
    write_stdout_line(&format!("Installed {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))?;
    make_executable(hook_path)?;
    configure_git_hooks_path(root, preflight)?;
    Ok(())
}

pub(crate) fn run_hook_uninstall(root: &Path) -> Result<(), String> {
    let preflight = HookInstallPreflight::load(root)?;
    let uses_canon_hooks_path = uses_canon_git_hooks_path(&preflight);
    if !uses_canon_hooks_path && has_canon_git_hooks_path(&preflight) {
        return Err(pre_commit_hook_manual_advice());
    }
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
