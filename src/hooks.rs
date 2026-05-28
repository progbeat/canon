use crate::fs_util::ensure_dir_without_symlinks;
use crate::git::resolve_git_path;
use crate::notes_cli::arg_to_string;
use crate::output::write_stdout_line;
use crate::platform;
use crate::project::command_output_trimmed;
use crate::{
    CHECK_PATH, DEFAULT_CHECK_TEMPLATE, DEFAULT_PRE_COMMIT_HOOK, GIT_HOOKS_PATH,
    PRE_COMMIT_HOOK_PATH,
};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

// User-facing output names the repository's pre-commit hook role at Git's
// default hook path. The managed script is stored under Canon's git-path state
// directory, and `core.hooksPath` points Git at that resolved hook directory.
const DEFAULT_GIT_PRE_COMMIT_HOOK_PATH: &str = ".git/hooks/pre-commit";
const PRE_COMMIT_HOOK_MANUAL_ADVICE: &str =
    "Can't safely install pre-commit hook.\n▷ Add `canon gate` manually to the existing hook setup or ask a human to handle it.";

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
    write_new_file(&check_path, DEFAULT_CHECK_TEMPLATE)?;
    write_stdout_line(&format!("Created {}", CHECK_PATH))?;
    Ok(())
}

fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

fn ensure_project_dir_without_symlinks(root: &Path, path: &Path) -> Result<(), String> {
    path.strip_prefix(root).map_err(|_| {
        format!(
            "refusing to create directory outside project root: {}",
            path.display()
        )
    })?;
    ensure_dir_without_symlinks(path)
}

fn write_new_file(path: &Path, content: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {}", path.display(), err))?;
    file.write_all(content.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

pub(crate) fn run_hook_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: canon hook install|uninstall".to_string());
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
    preflight_default_git_pre_commit_hook(root, &preflight)?;
    preflight_pre_commit_hook_content(preflight.pre_commit_hook.as_deref())?;
    preflight_git_hooks_path(&preflight)?;
    install_pre_commit_hook(root, &preflight)
}

pub(crate) fn run_hook_uninstall(root: &Path) -> Result<(), String> {
    let preflight = HookInstallPreflight::load(root)?;
    let hook_path = preflight.pre_commit_hook_path.clone();
    if let Some(existing) = preflight.pre_commit_hook.as_deref() {
        if !pre_commit_hook_is_reusable(existing) {
            return Err(pre_commit_hook_manual_advice());
        }
    }
    if uses_canon_git_hooks_path(&preflight) {
        unset_git_hooks_path(root)?;
    }
    if preflight.pre_commit_hook.is_some() {
        fs::remove_file(&hook_path)
            .map_err(|err| format!("failed to remove {}: {}", hook_path.display(), err))?;
    }
    write_stdout_line(&format!("Uninstalled {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

fn pre_commit_hook_manual_advice() -> String {
    PRE_COMMIT_HOOK_MANUAL_ADVICE.to_string()
}

fn preflight_default_git_pre_commit_hook(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    if uses_canon_git_hooks_path(preflight) {
        return Ok(());
    }
    // Canon-managed reusable hooks live under `PRE_COMMIT_HOOK_PATH` with
    // `core.hooksPath` pointing at `GIT_HOOKS_PATH`. The default Git hook path
    // is user-owned whenever Git is not already configured for Canon, even if a
    // file there happens to look compatible.
    if path_exists_no_follow(&root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))? {
        return Err(pre_commit_hook_manual_advice());
    }
    Ok(())
}

pub(crate) fn preflight_pre_commit_hook_content(content: Option<&str>) -> Result<(), String> {
    preflight_optional_hook_owner(content, pre_commit_hook_is_reusable)
}

pub(crate) fn preflight_git_hooks_path(preflight: &HookInstallPreflight) -> Result<(), String> {
    // Canon owns the hook directory only when Git has no custom hook manager or
    // already points at Canon's hook directory. Any other `core.hooksPath`
    // belongs to existing project Git integration and needs manual handling.
    preflight_optional_hook_owner(preflight.current_git_hooks_path.as_deref(), |existing| {
        git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing)
    })
}

fn preflight_optional_hook_owner(
    existing: Option<&str>,
    is_allowed: impl FnOnce(&str) -> bool,
) -> Result<(), String> {
    match existing {
        Some(value) if !is_allowed(value) => Err(pre_commit_hook_manual_advice()),
        _ => Ok(()),
    }
}

pub(crate) fn pre_commit_hook_is_reusable(content: &str) -> bool {
    content == DEFAULT_PRE_COMMIT_HOOK
}

pub(crate) fn install_pre_commit_hook(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    // The hook script is canon-owned persistent state, so it lives under the
    // repository's git-path state area. The local `core.hooksPath` value is Git
    // configuration: it points Git at that hook directory.
    let hook_path = &preflight.pre_commit_hook_path;
    if let Some(parent) = hook_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    if preflight.pre_commit_hook.as_deref() != Some(DEFAULT_PRE_COMMIT_HOOK) {
        write_new_file(&hook_path, DEFAULT_PRE_COMMIT_HOOK)?;
        write_stdout_line(&format!("Installed {}", DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))?;
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path(root, preflight)?;
    Ok(())
}

pub(crate) fn make_executable(path: &Path) -> Result<(), String> {
    platform::make_hook_executable(path)
}

pub(crate) fn current_git_hooks_path_for_worktree(root: &Path) -> Result<Option<String>, String> {
    let output = run_git_config_with_status(
        root,
        &["--local", "--get", "core.hooksPath"],
        "failed to read git core.hooksPath",
        |status| status.success() || status.code() == Some(1),
    )?;
    if output.status.success() {
        return Ok(Some(
            command_output_trimmed(&output.stdout, "git config stdout")?.to_string(),
        ));
    }
    Ok(None)
}

pub(crate) fn configure_git_hooks_path(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    if !preflight.is_git_worktree {
        write_stdout_line(&format!(
            "Git worktree not detected; {} was created but core.hooksPath was not set.",
            preflight.pre_commit_hook_path.display()
        ))?;
        return Ok(());
    }

    if uses_canon_git_hooks_path(preflight) {
        return Ok(());
    }

    set_git_hooks_path(root, preflight)
}

fn uses_canon_git_hooks_path(preflight: &HookInstallPreflight) -> bool {
    preflight
        .current_git_hooks_path
        .as_deref()
        .is_some_and(|existing| {
            git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing)
        })
}

pub(crate) fn set_git_hooks_path(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    let hooks_path = preflight.git_hooks_path.to_str().ok_or_else(|| {
        format!(
            "git hooks path must be valid UTF-8: {}",
            preflight.git_hooks_path.display()
        )
    })?;
    run_git_config_with_status(
        root,
        &["--local", "core.hooksPath", hooks_path],
        "failed to set git core.hooksPath",
        ExitStatus::success,
    )
    .map(|_| ())
}

pub(crate) fn unset_git_hooks_path(root: &Path) -> Result<(), String> {
    run_git_config_with_status(
        root,
        &["--local", "--unset", "core.hooksPath"],
        "failed to unset git core.hooksPath",
        |status| status.success() || status.code() == Some(5),
    )
    .map(|_| ())
}

fn run_git_config(root: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .args(args)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))
}

fn run_git_config_with_status(
    root: &Path,
    args: &[&str],
    failure_message: &str,
    accepts: impl FnOnce(&ExitStatus) -> bool,
) -> Result<Output, String> {
    let output = run_git_config(root, args)?;
    if accepts(&output.status) {
        return Ok(output);
    }
    Err(format!(
        "{}: {}",
        failure_message,
        command_output_trimmed(&output.stderr, "git config stderr")?
    ))
}

pub(crate) fn is_git_worktree(root: &Path) -> Result<bool, String> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to run git rev-parse: {}", err)),
    };
    Ok(output.status.success()
        && command_output_trimmed(&output.stdout, "git rev-parse stdout")? == "true")
}

pub(crate) struct HookInstallPreflight {
    pre_commit_hook: Option<String>,
    current_git_hooks_path: Option<String>,
    is_git_worktree: bool,
    root: PathBuf,
    git_hooks_path: PathBuf,
    pre_commit_hook_path: PathBuf,
}

impl HookInstallPreflight {
    pub(crate) fn load(root: &Path) -> Result<HookInstallPreflight, String> {
        let is_git_worktree = is_git_worktree(root)?;
        let git_hooks_path = canon_git_hooks_path(root, is_git_worktree)?;
        let pre_commit_hook_path = canon_pre_commit_hook_path(root, is_git_worktree)?;
        Ok(HookInstallPreflight {
            pre_commit_hook: read_optional_file(&pre_commit_hook_path)?,
            current_git_hooks_path: if is_git_worktree {
                current_git_hooks_path_for_worktree(root)?
            } else {
                None
            },
            is_git_worktree,
            root: root.to_path_buf(),
            git_hooks_path,
            pre_commit_hook_path,
        })
    }
}

fn canon_git_hooks_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    if is_git_worktree {
        return resolve_git_path(root, GIT_HOOKS_PATH);
    }
    Ok(root.join(".git").join(GIT_HOOKS_PATH))
}

fn canon_pre_commit_hook_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    if is_git_worktree {
        return resolve_git_path(root, PRE_COMMIT_HOOK_PATH);
    }
    Ok(root.join(".git").join(PRE_COMMIT_HOOK_PATH))
}

fn git_hooks_path_matches(root: &Path, expected: &Path, existing: &str) -> bool {
    let existing = Path::new(existing);
    if existing.is_absolute() {
        existing == expected
    } else {
        root.join(existing) == expected
    }
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {}", path.display(), err)),
    }
}
