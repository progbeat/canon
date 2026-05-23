use crate::config_types::AgentConfig;
use crate::git::git_path_bytes;
use crate::platform::checkout_index_prefix_arg;
use crate::project::command_output_trimmed;
use crate::scope::{effective_ignore_patterns, path_matches_pattern_bytes};
use crate::scope_hash::ScopeHashCache;
use crate::staged_worktree_git::run_git_command;
use crate::staged_worktree_paths::create_snapshot_root;
#[cfg(test)]
pub(crate) use crate::staged_worktree_paths::snapshot_parent_outside_worktree;
use crate::staged_worktree_validate::validate_snapshot_contains_no_symlinks;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct StagedWorktreeView {
    snapshot_root: PathBuf,
}

impl StagedWorktreeView {
    #[cfg(test)]
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let mut scope_hash_cache = ScopeHashCache::new();
        StagedWorktreeView::apply_with_scope_hash_cache(root, &mut scope_hash_cache)
    }

    pub(crate) fn apply_with_scope_hash_cache(
        root: &Path,
        scope_hash_cache: &mut ScopeHashCache,
    ) -> Result<StagedWorktreeView, String> {
        let snapshot_root = create_snapshot_root(root)?;
        if let Err(err) = materialize_staged_snapshot(root, &snapshot_root, scope_hash_cache) {
            let _ = fs::remove_dir_all(&snapshot_root);
            return Err(err);
        }
        Ok(StagedWorktreeView { snapshot_root })
    }

    pub(crate) fn snapshot_root(&self) -> &Path {
        &self.snapshot_root
    }

    pub(crate) fn remove_evaluator_denied_paths(&self, agent: &AgentConfig) -> Result<(), String> {
        let patterns = effective_ignore_patterns(agent);
        remove_evaluator_denied_snapshot_paths(
            &self.snapshot_root,
            &self.snapshot_root,
            &patterns,
        )?;
        if git_metadata_root_is_denied(&patterns) {
            return Ok(());
        }
        rebuild_snapshot_git_metadata(&self.snapshot_root)?;
        remove_denied_rebuilt_git_metadata_paths(&self.snapshot_root, &patterns)?;
        Ok(())
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.snapshot_root);
    }
}

fn materialize_staged_snapshot(
    root: &Path,
    snapshot_root: &Path,
    _scope_hash_cache: &mut ScopeHashCache,
) -> Result<(), String> {
    initialize_snapshot_git_repo(snapshot_root)?;
    checkout_staged_index(root, snapshot_root)?;
    stage_snapshot_index(snapshot_root)?;
    validate_snapshot_contains_no_symlinks(snapshot_root)
}

fn remove_evaluator_denied_snapshot_paths(
    snapshot_root: &Path,
    current: &Path,
    patterns: &[String],
) -> Result<(), String> {
    let entries = fs::read_dir(current).map_err(|err| {
        format!(
            "failed to inspect staged snapshot {}: {}",
            current.display(),
            err
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            format!(
                "failed to inspect staged snapshot entry under {}: {}",
                current.display(),
                err
            )
        })?;
        let path = entry.path();
        if snapshot_path_is_evaluator_denied(snapshot_root, &path, patterns)? {
            remove_snapshot_path(&path)?;
            continue;
        }
        if snapshot_path_is_git_metadata_root(snapshot_root, &path)? {
            continue;
        }
        let file_type = entry.file_type().map_err(|err| {
            format!(
                "failed to inspect staged snapshot path {}: {}",
                path.display(),
                err
            )
        })?;
        if file_type.is_dir() {
            remove_evaluator_denied_snapshot_paths(snapshot_root, &path, patterns)?;
        }
    }
    Ok(())
}

fn snapshot_path_is_git_metadata_root(snapshot_root: &Path, path: &Path) -> Result<bool, String> {
    let relative = path.strip_prefix(snapshot_root).map_err(|_| {
        format!(
            "staged snapshot path {} is outside {}",
            path.display(),
            snapshot_root.display()
        )
    })?;
    Ok(relative == Path::new(".git"))
}

fn git_metadata_root_is_denied(patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| path_matches_pattern_bytes(b".git", pattern.as_bytes()))
}

fn snapshot_path_is_evaluator_denied(
    snapshot_root: &Path,
    path: &Path,
    patterns: &[String],
) -> Result<bool, String> {
    let relative = path.strip_prefix(snapshot_root).map_err(|_| {
        format!(
            "staged snapshot path {} is outside {}",
            path.display(),
            snapshot_root.display()
        )
    })?;
    let mut relative_path = git_path_bytes(relative)?;
    normalize_path_separators(&mut relative_path);
    Ok(patterns
        .iter()
        .any(|pattern| path_matches_pattern_bytes(&relative_path, pattern.as_bytes())))
}

fn normalize_path_separators(path: &mut [u8]) {
    let separator = std::path::MAIN_SEPARATOR as u8;
    if separator != b'/' {
        for byte in path {
            if *byte == separator {
                *byte = b'/';
            }
        }
    }
}

fn remove_snapshot_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "failed to inspect evaluator-denied snapshot path {}: {}",
                path.display(),
                err
            ));
        }
    }
    .map_err(|err| {
        format!(
            "failed to remove evaluator-denied snapshot path {}: {}",
            path.display(),
            err
        )
    })
}

fn rebuild_snapshot_git_metadata(snapshot_root: &Path) -> Result<(), String> {
    let git_dir = snapshot_root.join(".git");
    remove_snapshot_path(&git_dir)?;
    initialize_snapshot_git_repo(snapshot_root)?;
    stage_snapshot_index(snapshot_root)
}

fn remove_denied_rebuilt_git_metadata_paths(
    snapshot_root: &Path,
    patterns: &[String],
) -> Result<(), String> {
    let mut recursive_patterns = Vec::new();
    for pattern in patterns {
        if remove_known_rebuilt_git_metadata_path_if_denied(snapshot_root, pattern)? {
            continue;
        }
        if deny_pattern_may_match_rebuilt_git_metadata(pattern) {
            recursive_patterns.push(pattern.clone());
        }
    }
    if recursive_patterns.is_empty() {
        return Ok(());
    }
    remove_evaluator_denied_snapshot_paths(
        snapshot_root,
        &snapshot_root.join(".git"),
        &recursive_patterns,
    )
}

fn remove_known_rebuilt_git_metadata_path_if_denied(
    snapshot_root: &Path,
    pattern: &str,
) -> Result<bool, String> {
    for relative in [".git/canon", ".git/canon/logs"] {
        if path_matches_pattern_bytes(relative.as_bytes(), pattern.as_bytes()) {
            remove_snapshot_path(&snapshot_root.join(relative))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn deny_pattern_may_match_rebuilt_git_metadata(pattern: &str) -> bool {
    let first_component = pattern.split('/').next().unwrap_or(pattern);
    path_matches_pattern_bytes(b".git", first_component.as_bytes())
}

fn checkout_staged_index(root: &Path, snapshot_root: &Path) -> Result<(), String> {
    // Keep staged symlinks as regular files containing the link target. That
    // prevents evaluator reads from following a tracked symlink out of the
    // staged snapshot while still showing the staged link target text.
    checkout_index_into_snapshot(
        root,
        snapshot_root,
        None,
        "failed to materialize staged snapshot",
    )
}

fn initialize_snapshot_git_repo(snapshot_root: &Path) -> Result<(), String> {
    let template = snapshot_root.join(".canon-empty-git-template");
    fs::create_dir(&template).map_err(|err| {
        format!(
            "failed to create empty Git template directory {}: {}",
            template.display(),
            err
        )
    })?;
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .arg("init")
            .arg("--quiet")
            .arg("--template")
            .arg(&template),
        "git init",
        "failed to initialize staged snapshot Git metadata",
    )?;
    let _ = fs::remove_dir(&template);
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.eol", "lf"),
        ("core.symlinks", "false"),
    ] {
        set_snapshot_git_config(snapshot_root, key, value)?;
    }
    Ok(())
}

#[cfg(all(test, unix, not(target_os = "macos")))]
pub(crate) fn initialize_snapshot_git_repo_for_test(snapshot_root: &Path) -> Result<(), String> {
    initialize_snapshot_git_repo(snapshot_root)
}

fn set_snapshot_git_config(snapshot_root: &Path, key: &str, value: &str) -> Result<(), String> {
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .args(["config", key, value]),
        "git config",
        "failed to configure staged snapshot Git metadata",
    )
}

fn checkout_index_into_snapshot(
    root: &Path,
    snapshot_root: &Path,
    index_file: Option<&Path>,
    failure_message: &str,
) -> Result<(), String> {
    let prefix = checkout_index_prefix(snapshot_root)?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("-c")
        .arg("core.symlinks=false")
        .arg("checkout-index")
        .arg("--all")
        .arg("--force")
        .arg(prefix);
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git checkout-index: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            failure_message,
            command_output_trimmed(&output.stderr, "git checkout-index stderr")?
        ));
    }
    Ok(())
}

fn stage_snapshot_index(snapshot_root: &Path) -> Result<(), String> {
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .args(["add", "--all", "--force"]),
        "git add",
        "failed to stage snapshot Git index",
    )
}

fn checkout_index_prefix(snapshot_root: &Path) -> Result<std::ffi::OsString, String> {
    checkout_index_prefix_arg(snapshot_root)
}
