use crate::notes::cli::LOCK_STALE_AFTER_SECS;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

pub(crate) fn stale_lock_age(age: Duration) -> bool {
    age >= Duration::from_secs(LOCK_STALE_AFTER_SECS)
}

pub(crate) fn create_lock_file(path: &Path) -> Result<fs::File, io::Error> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

pub(crate) fn remove_stale_lock(path: &Path) -> Result<(), String> {
    remove_stale_lock_with_permission_denied(path, false).map(|_| ())
}

#[cfg(not(unix))]
pub(crate) fn remove_stale_lock_for_retry(path: &Path) -> Result<bool, String> {
    remove_stale_lock_with_permission_denied(path, true)
}

fn remove_stale_lock_with_permission_denied(
    path: &Path,
    report_permission_denied_as_held: bool,
) -> Result<bool, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err)
            if err.kind() == io::ErrorKind::PermissionDenied
                && report_permission_denied_as_held =>
        {
            Ok(false)
        }
        Err(err) => Err(format!(
            "failed to remove stale lock {}: {}",
            path.display(),
            err
        )),
    }
}
