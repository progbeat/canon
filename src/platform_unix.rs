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
    let unix_mode = if mode == "100755" { 0o555 } else { 0o444 };
    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?
        .permissions();
    permissions.set_mode(unix_mode);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

pub(crate) fn set_materialized_dir_permissions(path: &Path) -> Result<(), String> {
    set_directory_permissions(path, 0o555)
}

pub(crate) fn set_private_dir_permissions(path: &Path) -> Result<(), String> {
    set_directory_permissions(path, 0o700)
}

pub(crate) fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if !metadata.file_type().is_file() {
        return Err(format!("refusing to chmod non-file {}", path.display()));
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

fn set_directory_permissions(path: &Path, mode: u32) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if !metadata.file_type().is_dir() {
        return Err(format!(
            "refusing to chmod non-directory {}",
            path.display()
        ));
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> Result<(), String> {
    let target = std::ffi::OsStr::from_bytes(target);
    symlink(target, link).map_err(|err| {
        format!(
            "failed to symlink evaluator file {}: {}",
            link.display(),
            err
        )
    })
}

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        format!(
            "failed to inspect evaluator file {}: {}",
            source.display(),
            err
        )
    })?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source)
            .map_err(|err| format!("failed to read symlink {}: {}", source.display(), err))?;
        symlink(&link_target, target).map_err(|err| {
            format!(
                "failed to copy evaluator symlink {} to {}: {}",
                source.display(),
                target.display(),
                err
            )
        })
    } else {
        fs::hard_link(source, target).map_err(|err| {
            format!(
                "failed to hardlink evaluator scope file {} to {}: {}",
                source.display(),
                target.display(),
                err
            )
        })
    }
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

pub(crate) fn add_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_discovered_memory_backed_staged_snapshot_parent_candidates(parents);
}

pub(crate) fn add_ordinary_staged_snapshot_parent_candidates(_parents: &mut Vec<PathBuf>) {}

fn add_discovered_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer memory-backed locations when the host exposes them. Missing
    // candidates are harmless: snapshot creation skips paths that do not exist
    // and later falls back to the ordinary temporary directory.
    for path in discover_memory_backed_mount_points() {
        push_unique_path(parents, path);
    }
}

fn discover_memory_backed_mount_points() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    add_mountinfo_memory_backed_paths(&mut paths);
    add_mounts_memory_backed_paths(&mut paths);
    paths
}

fn add_mountinfo_memory_backed_paths(paths: &mut Vec<PathBuf>) {
    let Ok(contents) = fs::read_to_string("/proc/self/mountinfo") else {
        return;
    };
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(separator) = fields.iter().position(|field| *field == "-") else {
            continue;
        };
        if separator <= 4 || fields.len() <= separator + 1 {
            continue;
        }
        if is_memory_backed_filesystem(fields[separator + 1]) {
            push_unique_path(paths, proc_mount_path(fields[4]));
        }
    }
}

fn add_mounts_memory_backed_paths(paths: &mut Vec<PathBuf>) {
    let Ok(contents) = fs::read_to_string("/proc/mounts") else {
        return;
    };
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 {
            continue;
        }
        if is_memory_backed_filesystem(fields[2]) {
            push_unique_path(paths, proc_mount_path(fields[1]));
        }
    }
}

fn is_memory_backed_filesystem(fs_type: &str) -> bool {
    matches!(fs_type, "tmpfs" | "ramfs")
}

fn proc_mount_path(raw: &str) -> PathBuf {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            if let Some(byte) = octal_escape_byte(&bytes[index + 1..index + 4]) {
                output.push(byte);
                index += 4;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    PathBuf::from(OsString::from_vec(output))
}

fn octal_escape_byte(digits: &[u8]) -> Option<u8> {
    if digits.len() != 3 || !digits.iter().all(|digit| matches!(digit, b'0'..=b'7')) {
        return None;
    }
    let value = u16::from(digits[0] - b'0') * 64
        + u16::from(digits[1] - b'0') * 8
        + u16::from(digits[2] - b'0');
    u8::try_from(value).ok()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> Result<PathBuf, String> {
    Ok(PathBuf::from(OsString::from_vec(bytes)))
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
