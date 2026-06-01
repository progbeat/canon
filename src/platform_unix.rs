use super::{push_unique_path, wait_for_app_server_child, CHECK_INTERRUPTED};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::mem;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

extern "C" fn handle_check_signal(_: i32) {
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

#[derive(Debug)]
pub(crate) enum PlatformError {
    Context {
        context: String,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    Related {
        context: &'static str,
        sources: Box<RelatedErrorSource>,
    },
}

#[derive(Debug)]
pub(crate) struct RelatedErrorSource {
    error: PlatformError,
    next: Option<Box<RelatedErrorSource>>,
}

type PlatformResult<T> = Result<T, PlatformError>;

impl PlatformError {
    fn message(context: impl Into<String>) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: None,
        }
    }

    fn io(context: impl Into<String>, source: io::Error) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }

    fn with_source(context: impl Into<String>, source: PlatformError) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }

    fn chain(mut errors: Vec<PlatformError>) -> Self {
        if errors.len() <= 1 {
            return errors
                .pop()
                .unwrap_or_else(|| PlatformError::message("unknown platform error"));
        }
        let mut sources = None;
        while let Some(error) = errors.pop() {
            sources = Some(Box::new(RelatedErrorSource {
                error,
                next: sources,
            }));
        }
        PlatformError::Related {
            context: "multiple platform errors",
            sources: sources.expect("multiple errors produced at least one source"),
        }
    }
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::Context { context, source } => match source {
                Some(source) => write!(formatter, "{}: {}", context, source),
                None => formatter.write_str(context),
            },
            PlatformError::Related { context, sources } => {
                write!(formatter, "{}: {}", context, sources)
            }
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PlatformError::Context { source, .. } => source
                .as_deref()
                .map(|source| source as &(dyn std::error::Error + 'static)),
            PlatformError::Related { sources, .. } => Some(sources),
        }
    }
}

impl std::fmt::Display for RelatedErrorSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)?;
        if let Some(next) = &self.next {
            write!(formatter, "; {}", next)?;
        }
        Ok(())
    }
}

impl std::error::Error for RelatedErrorSource {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.next
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

pub(crate) fn install_check_signal_handlers() -> PlatformResult<()> {
    for signal_number in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        install_signal_handler(signal_number)?;
    }
    Ok(())
}

fn install_signal_handler(signal_number: libc::c_int) -> PlatformResult<()> {
    // SAFETY: `sigaction` is initialized before use, the handler has C ABI and
    // only stores to an atomic flag, and libc reports failure via the return
    // value checked immediately below.
    let result = unsafe {
        let mut action: libc::sigaction = mem::zeroed();
        action.sa_flags = 0;
        action.sa_sigaction = handle_check_signal as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal_number, &action, std::ptr::null_mut())
    };
    if result == -1 {
        Err(PlatformError::io(
            format!(
                "failed to install signal handler for signal {}",
                signal_number
            ),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn prepare_app_server_command(command: &mut Command) {
    command.process_group(0);
}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> PlatformResult<()> {
    if poll_app_server_child(child)?.is_some() {
        return Ok(());
    }
    let process_group = app_server_process_group(child)?;
    let mut errors = Vec::new();
    signal_process_group_or_kill_child(child, process_group, libc::SIGTERM, &mut errors);
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    signal_process_group_or_kill_child(child, process_group, libc::SIGKILL, &mut errors);
    if let Err(err) = wait_for_app_server_child(child) {
        errors.push(PlatformError::message(err));
    }
    finish_app_server_cleanup(errors)
}

fn app_server_process_group(child: &Child) -> PlatformResult<libc::pid_t> {
    let pid = child.id();
    libc::pid_t::try_from(pid).map_err(|_| {
        PlatformError::message(format!(
            "app-server child pid {} does not fit Unix process group id",
            pid
        ))
    })
}

fn signal_process_group(
    process_group: libc::pid_t,
    signal_number: libc::c_int,
) -> PlatformResult<()> {
    // SAFETY: POSIX `kill` uses a negative pid to address a process group.
    // The app-server child is spawned in its own process group first.
    let result = unsafe { libc::kill(-process_group, signal_number) };
    if result == 0 {
        Ok(())
    } else {
        Err(PlatformError::io(
            format!(
                "failed to send signal {} to app-server process group {}",
                signal_number, process_group
            ),
            io::Error::last_os_error(),
        ))
    }
}

fn signal_process_group_or_kill_child(
    child: &mut Child,
    process_group: libc::pid_t,
    signal_number: libc::c_int,
    errors: &mut Vec<PlatformError>,
) {
    if let Err(err) = signal_process_group(process_group, signal_number) {
        if child_already_exited(child, errors) {
            return;
        }
        errors.push(err);
        if let Err(err) = child.kill() {
            if !child_already_exited(child, errors) {
                errors.push(PlatformError::io("failed to kill app-server child", err));
            }
        }
    }
}

fn child_already_exited(child: &mut Child, errors: &mut Vec<PlatformError>) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(err) => {
            errors.push(PlatformError::io("failed to poll app-server child", err));
            false
        }
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> PlatformResult<bool> {
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

fn poll_app_server_child(child: &mut Child) -> PlatformResult<Option<ExitStatus>> {
    child
        .try_wait()
        .map_err(|err| PlatformError::io("failed to poll app-server child", err))
}

fn finish_app_server_cleanup(errors: Vec<PlatformError>) -> PlatformResult<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::chain(errors))
    }
}

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> PlatformResult<()> {
    symlink(source, target).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to symlink evaluator CODEX_HOME file {} to {}",
                target.display(),
                source.display()
            ),
            err,
        )
    })
}

pub(crate) fn move_path(source: &Path, target: &Path) -> PlatformResult<()> {
    move_path_preserving_directory_permissions(source, target)
}

fn move_path_preserving_directory_permissions(source: &Path, target: &Path) -> PlatformResult<()> {
    let Some(directory) = open_source_directory_for_move(source)? else {
        return rename_path(source, target);
    };
    let mode = directory_permissions(source, &directory)?.mode();
    fchmod_open_path(source, &directory, 0o700)?;
    if let Err(rename_err) = rename_path(source, target) {
        return Err(restore_source_directory_permissions_after_failed_move(
            source, &directory, mode, rename_err,
        ));
    }
    if let Err(restore_err) = fchmod_open_path(target, &directory, mode) {
        return Err(rollback_moved_directory_after_restore_failure(
            source,
            target,
            &directory,
            mode,
            restore_err,
        ));
    }
    Ok(())
}

fn restore_source_directory_permissions_after_failed_move(
    source: &Path,
    directory: &fs::File,
    mode: u32,
    rename_err: PlatformError,
) -> PlatformError {
    match fchmod_open_path(source, directory, mode) {
        Ok(()) => rename_err,
        Err(restore_err) => PlatformError::chain(vec![rename_err, restore_err]),
    }
}

fn rollback_moved_directory_after_restore_failure(
    source: &Path,
    target: &Path,
    directory: &fs::File,
    mode: u32,
    restore_err: PlatformError,
) -> PlatformError {
    let mut errors = vec![restore_err];
    match rename_path(target, source) {
        Ok(()) => {
            if let Err(source_restore_err) = fchmod_open_path(source, directory, mode) {
                errors.push(source_restore_err);
            }
        }
        Err(rollback_err) => errors.push(PlatformError::with_source(
            "failed to roll back moved directory",
            rollback_err,
        )),
    }
    PlatformError::chain(errors)
}

fn rename_path(source: &Path, target: &Path) -> PlatformResult<()> {
    fs::rename(source, target).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to move isolated path {} to {}",
                source.display(),
                target.display()
            ),
            err,
        )
    })
}

pub(crate) fn make_hook_executable(path: &Path) -> PlatformResult<()> {
    let Some(file) = open_file_for_chmod(path, ChmodSymlink::Reject)? else {
        return Err(PlatformError::message(format!(
            "refusing to chmod symlink {}",
            path.display()
        )));
    };
    fchmod_open_path(path, &file, 0o755)
}

pub(crate) fn set_materialized_file_permissions(path: &Path, mode: &str) -> PlatformResult<()> {
    let Some(file) = open_file_for_chmod(path, ChmodSymlink::Ignore)? else {
        return Ok(());
    };
    let unix_mode = match mode {
        "100644" => 0o444,
        "100755" => 0o555,
        _ => {
            return Err(PlatformError::message(format!(
                "unsupported materialized file mode {} for {}",
                mode,
                path.display()
            )));
        }
    };
    fchmod_open_path(path, &file, unix_mode)
}

pub(crate) fn set_materialized_dir_permissions(path: &Path) -> PlatformResult<()> {
    set_directory_permissions(path, 0o555)
}

pub(crate) fn set_private_dir_permissions(path: &Path) -> PlatformResult<()> {
    set_directory_permissions(path, 0o700)
}

pub(crate) fn set_private_file_permissions(path: &Path) -> PlatformResult<()> {
    let Some(file) = open_file_for_chmod(path, ChmodSymlink::Ignore)? else {
        return Ok(());
    };
    fchmod_open_path(path, &file, 0o600)
}

#[derive(Clone)]
pub(crate) struct SecretDirMode {
    permissions: fs::Permissions,
    directory: Arc<fs::File>,
}

pub(crate) fn secret_dir_mode(path: &Path) -> PlatformResult<SecretDirMode> {
    let directory = open_directory_for_chmod(path)?;
    Ok(SecretDirMode {
        permissions: directory_permissions(path, &directory)?,
        directory: Arc::new(directory),
    })
}

pub(crate) fn chmod_secret_dir_no_access(path: &Path) -> PlatformResult<()> {
    set_directory_permissions(path, 0o000)
}

pub(crate) fn restore_secret_dir_mode(path: &Path, mode: &SecretDirMode) -> PlatformResult<()> {
    fchmod_open_path(path, &mode.directory, mode.permissions.mode())
}

fn set_directory_permissions(path: &Path, mode: u32) -> PlatformResult<()> {
    let directory = open_directory_for_chmod(path)?;
    fchmod_open_path(path, &directory, mode)
}

enum ChmodSymlink {
    Ignore,
    Reject,
}

fn open_source_directory_for_move(path: &Path) -> PlatformResult<Option<fs::File>> {
    match open_directory_no_follow(path) {
        Ok(directory) => Ok(Some(directory)),
        Err(err) if path_error_is_not_directory_or_is_symlink(&err) => Ok(None),
        Err(err) => Err(PlatformError::io(
            format!("failed to open directory {}", path.display()),
            err,
        )),
    }
}

fn open_directory_for_chmod(path: &Path) -> PlatformResult<fs::File> {
    let directory = open_directory_no_follow(path).map_err(|err| {
        if path_error_is_symlink(&err) {
            PlatformError::message(format!("refusing to chmod symlink {}", path.display()))
        } else {
            PlatformError::io(format!("failed to open directory {}", path.display()), err)
        }
    })?;
    let metadata = directory.metadata().map_err(|err| {
        PlatformError::io(format!("failed to inspect opened {}", path.display()), err)
    })?;
    if !metadata.file_type().is_dir() {
        return Err(PlatformError::message(format!(
            "refusing to chmod non-directory {}",
            path.display()
        )));
    }
    Ok(directory)
}

fn open_file_for_chmod(path: &Path, symlink: ChmodSymlink) -> PlatformResult<Option<fs::File>> {
    let mut options = fs::OpenOptions::new();
    options.read(true).custom_flags(libc::O_NOFOLLOW);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(err) if path_error_is_symlink(&err) && matches!(symlink, ChmodSymlink::Ignore) => {
            return Ok(None);
        }
        Err(err) if path_error_is_symlink(&err) => {
            return Err(PlatformError::message(format!(
                "refusing to chmod symlink {}",
                path.display()
            )));
        }
        Err(err) => {
            return Err(PlatformError::io(
                format!("failed to open {}", path.display()),
                err,
            ));
        }
    };
    let metadata = file.metadata().map_err(|err| {
        PlatformError::io(format!("failed to inspect opened {}", path.display()), err)
    })?;
    if !metadata.file_type().is_file() {
        return Err(PlatformError::message(format!(
            "refusing to chmod non-file {}",
            path.display()
        )));
    }
    Ok(Some(file))
}

fn open_directory_no_follow(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    options.open(path)
}

fn directory_permissions(path: &Path, directory: &fs::File) -> PlatformResult<fs::Permissions> {
    directory
        .metadata()
        .map(|metadata| metadata.permissions())
        .map_err(|err| {
            PlatformError::io(format!("failed to inspect opened {}", path.display()), err)
        })
}

fn fchmod_open_path(path: &Path, file: &fs::File, mode: u32) -> PlatformResult<()> {
    let result = unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) };
    if result == 0 {
        Ok(())
    } else {
        Err(PlatformError::io(
            format!("failed to chmod {}", path.display()),
            io::Error::last_os_error(),
        ))
    }
}

fn path_error_is_not_directory_or_is_symlink(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(code) if code == libc::ENOTDIR || code == libc::ELOOP
    )
}

fn path_error_is_symlink(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::ELOOP)
}

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> PlatformResult<()> {
    let target = std::ffi::OsStr::from_bytes(target);
    symlink(target, link).map_err(|err| {
        PlatformError::io(
            format!("failed to symlink evaluator file {}", link.display()),
            err,
        )
    })
}

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> PlatformResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        PlatformError::io(
            format!("failed to inspect evaluator file {}", source.display()),
            err,
        )
    })?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source).map_err(|err| {
            PlatformError::io(format!("failed to read symlink {}", source.display()), err)
        })?;
        symlink(&link_target, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to copy evaluator symlink {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        })
    } else {
        fs::hard_link(source, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to hardlink evaluator scope file {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
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
) -> PlatformResult<fs::File> {
    fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|err| PlatformError::io(format!("failed to open {}", path.display()), err))
}

pub(crate) fn add_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_discovered_memory_backed_staged_snapshot_parent_candidates(parents);
}

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

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

pub(crate) fn git_path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    OsString::from_vec(bytes)
}

#[cfg(test)]
pub(crate) fn git_path_from_raw_bytes(path: &[u8]) -> OsString {
    std::ffi::OsStr::from_bytes(path).to_os_string()
}
