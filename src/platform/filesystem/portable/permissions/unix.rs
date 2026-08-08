use super::super::unix::{PlatformError, PlatformResult};
use super::MaterializedFileMode;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(in super::super) fn make_hook_executable(path: &Path) -> PlatformResult<()> {
    let Some(file) = open_file_for_chmod(path, ChmodSymlink::Reject)? else {
        return Err(PlatformError::message(format!(
            "refusing to chmod symlink {}",
            path.display()
        )));
    };
    fchmod_open_path(path, &file, 0o755)
}

pub(in super::super) fn set_materialized_file_permissions(
    path: &Path,
    mode: MaterializedFileMode,
) -> PlatformResult<()> {
    let unix_mode = match mode {
        MaterializedFileMode::ReadOnly => 0o444,
        MaterializedFileMode::Executable => 0o555,
        MaterializedFileMode::Symlink => return Ok(()),
    };
    let Some(file) = open_file_for_chmod(path, ChmodSymlink::Ignore)? else {
        return Ok(());
    };
    fchmod_open_path(path, &file, unix_mode)
}

pub(in super::super) fn set_materialized_dir_permissions(path: &Path) -> PlatformResult<()> {
    set_directory_permissions(path, 0o555)
}

#[derive(Clone)]
pub(in super::super) struct SecretDirMode {
    permissions: fs::Permissions,
}

pub(in super::super) fn secret_dir_mode(path: &Path) -> PlatformResult<SecretDirMode> {
    fs::metadata(path)
        .map(|metadata| SecretDirMode {
            permissions: metadata.permissions(),
        })
        .map_err(|err| {
            PlatformError::io(
                format!("failed to inspect secret directory {}", path.display()),
                err,
            )
        })
}

pub(in super::super) fn chmod_secret_dir_no_access(path: &Path) -> PlatformResult<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).map_err(|err| {
        PlatformError::io(
            format!("failed to chmod secret directory {}", path.display()),
            err,
        )
    })
}

pub(in super::super) fn restore_secret_dir_mode(
    path: &Path,
    mode: &SecretDirMode,
) -> PlatformResult<()> {
    fs::set_permissions(path, mode.permissions.clone()).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to restore secret directory permissions {}",
                path.display()
            ),
            err,
        )
    })
}

fn set_directory_permissions(path: &Path, mode: u32) -> PlatformResult<()> {
    let directory = open_directory_for_chmod(path)?;
    fchmod_open_path(path, &directory, mode)
}

enum ChmodSymlink {
    Ignore,
    Reject,
}

pub(in super::super) fn open_source_directory_for_move(
    path: &Path,
) -> PlatformResult<Option<fs::File>> {
    match open_directory_no_follow(path) {
        Ok(directory) => Ok(Some(directory)),
        Err(err) if path_error_is_not_directory_or_is_symlink(&err) => Ok(None),
        Err(err) => Err(PlatformError::io(
            format!("failed to open directory {}", path.display()),
            err,
        )),
    }
}

pub(super) fn open_directory_for_chmod(path: &Path) -> PlatformResult<fs::File> {
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

pub(super) fn open_directory_no_follow(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    options.open(path)
}

pub(in super::super) fn directory_permissions(
    path: &Path,
    directory: &fs::File,
) -> PlatformResult<fs::Permissions> {
    directory
        .metadata()
        .map(|metadata| metadata.permissions())
        .map_err(|err| {
            PlatformError::io(format!("failed to inspect opened {}", path.display()), err)
        })
}

pub(in super::super) fn fchmod_open_path(
    path: &Path,
    file: &fs::File,
    mode: u32,
) -> PlatformResult<()> {
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
