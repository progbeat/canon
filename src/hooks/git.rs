use super::fs::read_optional_file;
use super::{
    DEFAULT_GIT_PRE_COMMIT_HOOK_PATH, DEFAULT_PRE_COMMIT_GIT_PATH, GIT_HOOKS_PATH,
    GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL, PRE_COMMIT_HOOK_PATH,
};
use crate::git::resolve_git_path;
use crate::platform;
use crate::project::command_output_trimmed;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

fn current_git_hooks_paths_for_worktree(root: &Path) -> Result<Vec<String>, String> {
    let output = run_git_config_with_status(
        root,
        &["--local", "--get-all", "core.hooksPath"],
        "failed to read git core.hooksPath",
        |status| status.success() || status.code() == Some(1),
    )?;
    if output.status.success() {
        return command_output_lines(&output.stdout, "git config stdout");
    }
    Ok(Vec::new())
}

fn command_output_lines(output: &[u8], description: &str) -> Result<Vec<String>, String> {
    let mut text = String::from_utf8(output.to_vec())
        .map_err(|err| format!("{} was not valid UTF-8: {}", description, err))?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.ends_with('\n') {
        text.pop();
    }
    Ok(text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect())
}

pub(super) fn configure_git_hooks_path(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    if !preflight.is_git_worktree {
        return Err(GIT_WORKTREE_REQUIRED_FOR_HOOK_INSTALL.to_string());
    }

    if uses_canon_git_hooks_path(preflight) {
        return Ok(());
    }

    set_git_hooks_path(root, preflight)
}

pub(super) fn uses_canon_git_hooks_path(preflight: &HookInstallPreflight) -> bool {
    !preflight.current_git_hooks_paths.is_empty()
        && preflight.current_git_hooks_paths.iter().all(|existing| {
            git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing)
        })
}

pub(super) fn has_canon_git_hooks_path(preflight: &HookInstallPreflight) -> bool {
    preflight.current_git_hooks_paths.iter().any(|existing| {
        git_hooks_path_matches(&preflight.root, &preflight.git_hooks_path, existing)
    })
}

pub(super) fn set_git_hooks_path(
    root: &Path,
    preflight: &HookInstallPreflight,
) -> Result<(), String> {
    let hooks_path = git_hooks_path_config_value(root, &preflight.git_hooks_path)?;
    run_git_config_with_status(
        root,
        &["--local", "core.hooksPath", &hooks_path],
        "failed to set git core.hooksPath",
        ExitStatus::success,
    )
    .map(|_| ())
}

fn git_hooks_path_config_value(root: &Path, path: &Path) -> Result<String, String> {
    let config_path = path.strip_prefix(root).unwrap_or(path);
    config_path.to_str().map(str::to_string).ok_or_else(|| {
        format!(
            "git hooks path must be valid UTF-8: {}",
            config_path.display()
        )
    })
}

pub(super) fn unset_git_hooks_path(root: &Path) -> Result<(), String> {
    run_git_config_with_status(
        root,
        &["--local", "--unset-all", "core.hooksPath"],
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

pub(super) fn is_git_worktree(root: &Path) -> Result<bool, String> {
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

pub(super) struct HookInstallPreflight {
    pub(super) pre_commit_hook: Option<String>,
    pub(super) current_git_hooks_paths: Vec<String>,
    pub(super) default_pre_commit_hook: Option<String>,
    pub(super) is_git_worktree: bool,
    pub(super) root: PathBuf,
    pub(super) git_hooks_path: PathBuf,
    pub(super) pre_commit_hook_path: PathBuf,
    pub(super) default_pre_commit_hook_path: PathBuf,
}

impl HookInstallPreflight {
    pub(super) fn load(root: &Path) -> Result<HookInstallPreflight, String> {
        let is_git_worktree = is_git_worktree(root)?;
        let git_hooks_path = canon_git_hooks_path(root, is_git_worktree)?;
        let pre_commit_hook_path = canon_pre_commit_hook_path(root, is_git_worktree)?;
        let default_pre_commit_hook_path = default_git_pre_commit_hook_path(root, is_git_worktree)?;
        let current_git_hooks_paths = if is_git_worktree {
            current_git_hooks_paths_for_worktree(root)?
        } else {
            Vec::new()
        };
        Ok(HookInstallPreflight {
            pre_commit_hook: read_optional_file(&pre_commit_hook_path)?,
            current_git_hooks_paths,
            default_pre_commit_hook: read_optional_file(&default_pre_commit_hook_path)?,
            is_git_worktree,
            root: root.to_path_buf(),
            git_hooks_path,
            pre_commit_hook_path,
            default_pre_commit_hook_path,
        })
    }
}

fn canon_git_hooks_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    canon_state_git_path(root, is_git_worktree, GIT_HOOKS_PATH)
}

fn canon_pre_commit_hook_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    canon_state_git_path(root, is_git_worktree, PRE_COMMIT_HOOK_PATH)
}

fn canon_state_git_path(
    root: &Path,
    is_git_worktree: bool,
    git_path: &str,
) -> Result<PathBuf, String> {
    if is_git_worktree {
        return resolve_git_path(root, git_path);
    }
    Ok(root.join(".git").join(git_path))
}

fn default_git_pre_commit_hook_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    if is_git_worktree {
        // Do not use `git rev-parse --git-path hooks/pre-commit` here:
        // while `core.hooksPath` points at Canon's managed hook directory,
        // Git resolves that query to the active managed hook path. Uninstall
        // needs the default hook path that will become active after
        // `core.hooksPath` is unset, which is under Git's common dir.
        return Ok(git_common_dir_path(root)?.join(DEFAULT_PRE_COMMIT_GIT_PATH));
    }
    Ok(root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

fn git_common_dir_path(root: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve git common dir: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    Ok(root.join(platform::path_from_git_stdout(output.stdout)?))
}

pub(super) fn git_hooks_path_matches(root: &Path, expected: &Path, existing: &str) -> bool {
    let existing = Path::new(existing);
    let existing = if existing.is_absolute() {
        existing.to_path_buf()
    } else {
        root.join(existing)
    };
    normalize_path_lexically(&existing) == normalize_path_lexically(expected)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}
