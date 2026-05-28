use super::wait_for_app_server_child;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    Ok(())
}

pub(crate) fn prepare_app_server_command(_command: &mut Command) {}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|err| format!("failed to kill app-server child: {}", err))?;
    wait_for_app_server_child(child)?;
    Ok(())
}

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "failed to copy evaluator CODEX_HOME file {} from {}: {}",
            target.display(),
            source.display(),
            err
        )
    })
}

pub(crate) fn make_hook_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_materialized_file_permissions(_path: &Path, _mode: &str) -> Result<(), String> {
    Ok(())
}

pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

pub(crate) fn open_file_for_append_without_following_symlink(
    path: &Path,
) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(crate) fn add_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_environment_temp_parent_candidates(parents);
}

fn add_environment_temp_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Non-Unix platforms have no portable mount table or standard RAM-backed
    // temp path. If the host provides one through the standard temp
    // environment variables, try it before the platform fallback temp dir.
    for name in ["TMPDIR", "TEMP", "TMP"] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if !value.is_empty() {
            push_unique_path(parents, PathBuf::from(value));
        }
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).to_string())
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    Ok(path
        .to_str()
        .ok_or_else(|| format!("git path must be valid UTF-8: {}", path.display()))?
        .as_bytes()
        .to_vec())
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<std::ffi::OsString, String> {
    String::from_utf8(bytes)
        .map(std::ffi::OsString::from)
        .map_err(|err| format!("path argument must be valid UTF-8: {}", err))
}
