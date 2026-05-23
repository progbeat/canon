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
use std::path::Path;
use std::process::Command;

// User-facing output names the repository's pre-commit hook role. The managed
// script is stored at `PRE_COMMIT_HOOK_PATH` under `.git/canon`, and
// `core.hooksPath` points Git at that directory.
const PRE_COMMIT_HOOK_DISPLAY_PATH: &str = ".git/hooks/pre-commit";
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
        ensure_dir_without_symlinks(root, parent)?;
    }
    write_new_file(&check_path, DEFAULT_CHECK_TEMPLATE)?;
    write_stdout_line(&format!("Created {}", CHECK_PATH))?;
    Ok(())
}

fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

fn ensure_dir_without_symlinks(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "refusing to create directory outside project root: {}",
            path.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(format!(
                        "refusing to use symlink directory {}",
                        current.display()
                    ));
                }
                if !metadata.is_dir() {
                    return Err(format!(
                        "{} exists but is not a directory",
                        current.display()
                    ));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|err| format!("failed to create {}: {}", current.display(), err))?;
            }
            Err(err) => {
                return Err(format!("failed to inspect {}: {}", current.display(), err));
            }
        }
    }
    Ok(())
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
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(existing) = preflight.pre_commit_hook.as_deref() {
        if !pre_commit_hook_is_reusable(existing) {
            return Err(pre_commit_hook_manual_advice());
        }
        fs::remove_file(&hook_path)
            .map_err(|err| format!("failed to remove {}: {}", hook_path.display(), err))?;
    }
    if preflight.current_git_hooks_path.as_deref() == Some(GIT_HOOKS_PATH) {
        unset_git_hooks_path(root)?;
    }
    write_stdout_line(&format!("Uninstalled {}", PRE_COMMIT_HOOK_DISPLAY_PATH))
}

fn pre_commit_hook_manual_advice() -> String {
    PRE_COMMIT_HOOK_MANUAL_ADVICE.to_string()
}

fn preflight_default_git_pre_commit_hook(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    if preflight.current_git_hooks_path.as_deref() == Some(GIT_HOOKS_PATH) {
        return Ok(());
    }
    if path_exists_no_follow(&root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))? {
        return Err(pre_commit_hook_manual_advice());
    }
    Ok(())
}

pub(crate) fn preflight_pre_commit_hook_content(content: Option<&str>) -> Result<(), String> {
    if let Some(existing) = content {
        if pre_commit_hook_is_reusable(existing) {
            return Ok(());
        }
        return Err(pre_commit_hook_manual_advice());
    }
    Ok(())
}

pub(crate) fn preflight_git_hooks_path(preflight: &HookInstallPreflight) -> Result<(), String> {
    // Canon owns the hook directory only when Git has no custom hook manager or
    // already points at Canon's hook directory. Any other `core.hooksPath`
    // belongs to existing project Git integration and needs manual handling.
    if let Some(existing) = preflight.current_git_hooks_path.as_deref() {
        if existing == GIT_HOOKS_PATH {
            return Ok(());
        }
        return Err(pre_commit_hook_manual_advice());
    }
    Ok(())
}

pub(crate) fn pre_commit_hook_is_reusable(content: &str) -> bool {
    content == DEFAULT_PRE_COMMIT_HOOK
}

pub(crate) fn install_pre_commit_hook(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    // The hook script is canon-owned persistent state, so it lives under the
    // repository's `.git/canon` state area. The local `core.hooksPath` value is
    // Git configuration: it points Git at that hook directory.
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = hook_path.parent() {
        ensure_dir_without_symlinks(root, parent)?;
    }
    if preflight.pre_commit_hook.as_deref() != Some(DEFAULT_PRE_COMMIT_HOOK) {
        write_new_file(&hook_path, DEFAULT_PRE_COMMIT_HOOK)?;
        write_stdout_line(&format!("Installed {}", PRE_COMMIT_HOOK_DISPLAY_PATH))?;
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path(root, preflight)?;
    Ok(())
}

pub(crate) fn make_executable(path: &Path) -> Result<(), String> {
    platform::make_hook_executable(path)
}

pub(crate) fn current_git_hooks_path_for_worktree(root: &Path) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("--get")
        .arg("core.hooksPath")
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(Some(
            command_output_trimmed(&output.stdout, "git config stdout")?.to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "failed to read git core.hooksPath: {}",
        command_output_trimmed(&output.stderr, "git config stderr")?
    ))
}

pub(crate) fn configure_git_hooks_path(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    if !preflight.is_git_worktree {
        write_stdout_line(&format!(
            "Git worktree not detected; {} was created but core.hooksPath was not set.",
            PRE_COMMIT_HOOK_PATH
        ))?;
        return Ok(());
    }

    if preflight.current_git_hooks_path.as_deref() == Some(GIT_HOOKS_PATH) {
        return Ok(());
    }

    set_git_hooks_path(root)
}

pub(crate) fn set_git_hooks_path(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("core.hooksPath")
        .arg(GIT_HOOKS_PATH)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "failed to set git core.hooksPath: {}",
        command_output_trimmed(&output.stderr, "git config stderr")?
    ))
}

pub(crate) fn unset_git_hooks_path(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("--unset")
        .arg("core.hooksPath")
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() || output.status.code() == Some(5) {
        return Ok(());
    }
    Err(format!(
        "failed to unset git core.hooksPath: {}",
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
}

impl HookInstallPreflight {
    pub(crate) fn load(root: &Path) -> Result<HookInstallPreflight, String> {
        let is_git_worktree = is_git_worktree(root)?;
        Ok(HookInstallPreflight {
            pre_commit_hook: read_optional_file(&root.join(PRE_COMMIT_HOOK_PATH))?,
            current_git_hooks_path: if is_git_worktree {
                current_git_hooks_path_for_worktree(root)?
            } else {
                None
            },
            is_git_worktree,
        })
    }
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {}", path.display(), err)),
    }
}
