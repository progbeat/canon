use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(unix))]
#[path = "platform_other.rs"]
mod platform_other;
#[cfg(unix)]
#[path = "platform_unix.rs"]
mod platform_unix;

#[cfg(not(unix))]
use platform_other as imp;
#[cfg(unix)]
use platform_unix as imp;

static CHECK_INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    imp::install_check_signal_handlers()
}

pub(crate) fn reset_check_interrupted() {
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) fn prepare_app_server_command(command: &mut Command) {
    imp::prepare_app_server_command(command);
}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    imp::terminate_app_server_child(child)
}

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    imp::mirror_evaluator_codex_home_file(source, target)
}

pub(crate) fn make_hook_executable(path: &Path) -> Result<(), String> {
    imp::make_hook_executable(path)
}

pub(crate) fn staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    imp::add_staged_snapshot_parent_candidates(&mut parents);
    let temp_dir = std::env::temp_dir();
    if !parents.iter().any(|parent| parent == &temp_dir) {
        parents.push(temp_dir);
    }
    parents
}

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> PathBuf {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    imp::path_from_git_bytes(bytes)
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    imp::git_path_bytes(path)
}

#[cfg(test)]
pub(crate) fn checkout_index_prefix_arg(path: &Path) -> Result<OsString, String> {
    let mut prefix = git_path_bytes(path)?;
    let separator = std::path::MAIN_SEPARATOR as u8;
    if prefix.last() != Some(&separator) {
        prefix.push(separator);
    }
    let mut arg = b"--prefix=".to_vec();
    arg.extend(prefix);
    imp::os_string_from_bytes(arg)
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    imp::os_string_from_bytes(bytes)
}

#[cfg(all(test, unix))]
pub(crate) fn git_path_from_raw_bytes(path: &[u8]) -> Result<std::ffi::OsString, String> {
    imp::git_path_from_raw_bytes(path)
}

fn wait_for_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|err| format!("failed to wait for app-server child: {}", err))
}
