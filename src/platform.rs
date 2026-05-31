use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(any(unix, windows)))]
compile_error!("canon requires Unix or Windows filesystem support");

#[cfg(windows)]
#[path = "platform_other.rs"]
mod platform_other;
#[cfg(unix)]
#[path = "platform_unix.rs"]
mod platform_unix;

#[cfg(windows)]
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

pub(crate) fn move_path(source: &Path, target: &Path) -> Result<(), String> {
    imp::move_path(source, target)
}

pub(crate) fn make_hook_executable(path: &Path) -> Result<(), String> {
    imp::make_hook_executable(path)
}

pub(crate) fn set_materialized_file_permissions(path: &Path, mode: &str) -> Result<(), String> {
    imp::set_materialized_file_permissions(path, mode)
}

pub(crate) fn set_materialized_dir_permissions(path: &Path) -> Result<(), String> {
    imp::set_materialized_dir_permissions(path)
}

pub(crate) fn set_private_dir_permissions(path: &Path) -> Result<(), String> {
    imp::set_private_dir_permissions(path)
}

pub(crate) fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    imp::set_private_file_permissions(path)
}

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> Result<(), String> {
    imp::create_materialized_symlink(target, link)
}

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    imp::hardlink_file_or_copy_symlink(source, target)
}

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    imp::create_private_dir(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    imp::create_private_dir_all(path)
}

pub(crate) fn open_file_for_append_without_following_symlink(
    path: &Path,
) -> Result<std::fs::File, String> {
    imp::open_file_for_append_without_following_symlink(path)
}

#[cfg(all(test, unix))]
pub(crate) fn staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = memory_backed_staged_snapshot_parent_candidates();
    for parent in ordinary_staged_snapshot_parent_candidates() {
        push_unique_path(&mut parents, parent);
    }
    parents
}

pub(crate) fn memory_backed_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    add_common_memory_backed_staged_snapshot_parent_candidates(&mut parents);
    imp::add_memory_backed_staged_snapshot_parent_candidates(&mut parents);
    parents
}

pub(crate) fn ordinary_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    imp::add_ordinary_staged_snapshot_parent_candidates(&mut parents);
    let temp_dir = std::env::temp_dir();
    if !parents.iter().any(|parent| parent == &temp_dir) {
        parents.push(temp_dir);
    }
    parents
}

fn add_common_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer common RAM-backed locations before ordinary temp directories.
    // Missing paths are skipped later by snapshot creation.
    push_unique_path(parents, PathBuf::from("/dev/shm"));
    push_unique_path(parents, PathBuf::from("/run/shm"));
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> Result<PathBuf, String> {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    imp::path_from_git_bytes(bytes)
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    imp::git_path_bytes(path)
}

#[cfg(all(test, unix))]
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
