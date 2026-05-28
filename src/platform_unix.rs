use super::{wait_for_app_server_child, CHECK_INTERRUPTED};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;
const SIGNAL_ERROR: usize = usize::MAX;

unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
}

extern "C" fn handle_check_signal(_: i32) {
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    for signal_number in [SIGHUP, SIGINT, SIGTERM] {
        install_signal_handler(signal_number)?;
    }
    Ok(())
}

fn install_signal_handler(signal_number: i32) -> Result<(), String> {
    // SAFETY: `handle_check_signal` has C ABI and only stores to an atomic
    // flag; the platform sentinel is checked immediately after registration.
    let previous = unsafe { signal(signal_number, handle_check_signal) };
    if previous == SIGNAL_ERROR {
        Err(format!(
            "failed to install signal handler for signal {}",
            signal_number
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn prepare_app_server_command(command: &mut Command) {
    command.process_group(0);
}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    if poll_app_server_child(child)?.is_some() {
        return Ok(());
    }
    let process_group = child.id() as i32;
    let mut errors = Vec::new();
    signal_process_group_or_kill_child(child, process_group, SIGTERM, &mut errors);
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    signal_process_group_or_kill_child(child, process_group, SIGKILL, &mut errors);
    wait_for_app_server_child(child)?;
    finish_app_server_cleanup(errors)
}

fn signal_process_group(process_group: i32, signal_number: i32) -> Result<(), String> {
    // SAFETY: POSIX `kill` uses a negative pid to address a process group.
    // The app-server child is spawned in its own process group first.
    let result = unsafe { kill(-process_group, signal_number) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "failed to send signal {} to app-server process group {}: {}",
            signal_number,
            process_group,
            io::Error::last_os_error()
        ))
    }
}

fn signal_process_group_or_kill_child(
    child: &mut Child,
    process_group: i32,
    signal_number: i32,
    errors: &mut Vec<String>,
) {
    if let Err(err) = signal_process_group(process_group, signal_number) {
        errors.push(err);
        if let Err(err) = child.kill() {
            errors.push(format!("failed to kill app-server child: {}", err));
        }
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if poll_app_server_child(child)?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn poll_app_server_child(child: &mut Child) -> Result<Option<ExitStatus>, String> {
    child
        .try_wait()
        .map_err(|err| format!("failed to poll app-server child: {}", err))
}

fn finish_app_server_cleanup(errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    symlink(source, target).map_err(|err| {
        format!(
            "failed to symlink evaluator CODEX_HOME file {} to {}: {}",
            target.display(),
            source.display(),
            err
        )
    })
}

pub(crate) fn make_hook_executable(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to chmod symlink {}", path.display()));
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

pub(crate) fn set_materialized_file_permissions(path: &Path, mode: &str) -> Result<(), String> {
    let unix_mode = if mode == "100755" { 0o755 } else { 0o644 };
    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?
        .permissions();
    permissions.set_mode(unix_mode);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    private_dir_builder(false).create(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    private_dir_builder(true).create(path)
}

fn private_dir_builder(recursive: bool) -> fs::DirBuilder {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    builder.mode(0o700);
    builder
}

pub(crate) fn open_file_for_append_without_following_symlink(
    path: &Path,
) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(crate) fn add_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_memory_backed_staged_snapshot_parent_candidates(parents);
}

fn add_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer common tmpfs-backed locations when the host exposes them. Missing
    // candidates are harmless: snapshot creation skips paths that do not exist
    // and later falls back to the ordinary temporary directory.
    parents.push(PathBuf::from("/dev/shm"));
    parents.push(PathBuf::from("/run/shm"));
}

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    Ok(path.as_os_str().as_bytes().to_vec())
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    Ok(OsString::from_vec(bytes))
}

#[cfg(test)]
pub(crate) fn git_path_from_raw_bytes(path: &[u8]) -> Result<OsString, String> {
    Ok(std::ffi::OsStr::from_bytes(path).to_os_string())
}
