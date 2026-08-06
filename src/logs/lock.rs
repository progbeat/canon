use crate::fs_util::reject_symlink;
use crate::logs::error::{log_io_error, DiagnosticLogResult};
use crate::logs::fs::remove_file_if_exists;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DIAGNOSTIC_LOG_LOCK_STALE_AFTER_SECS: u64 = 300;

// [l,g2] This file-backed lock is an ephemeral cross-process exclusion
// mechanism, not retained command state. Its token-owning guard removes it on
// drop; a later acquirer removes a crash leftover after the stale threshold.
pub(crate) struct EphemeralDiagnosticLogLock {
    path: PathBuf,
    token: String,
    file: Option<fs::File>,
}

impl Drop for EphemeralDiagnosticLogLock {
    fn drop(&mut self) {
        drop(self.file.take());
        if fs::read_to_string(&self.path).ok().as_deref() == Some(self.token.as_str()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn acquire_ephemeral_diagnostic_log_lock(
    log_dir: &Path,
) -> DiagnosticLogResult<EphemeralDiagnosticLogLock> {
    let path = ephemeral_lock_path_outside_bounded_log_dir(log_dir);
    match create_diagnostic_log_lock(&path) {
        Ok((token, file)) => Ok(ephemeral_diagnostic_log_lock(path, token, file)),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if diagnostic_log_lock_is_stale(&path)? {
                remove_file_if_exists(&path)?;
                let (token, file) = create_diagnostic_log_lock(&path)
                    .map_err(|err| log_io_error("lock", &path, err))?;
                Ok(ephemeral_diagnostic_log_lock(path, token, file))
            } else {
                Err(log_io_error("lock", &path, err))
            }
        }
        Err(err) => Err(log_io_error("lock", &path, err)),
    }
}

// [fh] Coordination metadata lives beside, rather than inside, the bounded
// retained-log directory. A live lock therefore neither consumes log quota
// nor becomes a pruning candidate; its exact sibling name remains internal.
fn ephemeral_lock_path_outside_bounded_log_dir(log_dir: &Path) -> PathBuf {
    log_dir.with_extension("lock")
}

fn ephemeral_diagnostic_log_lock(
    path: PathBuf,
    token: String,
    file: fs::File,
) -> EphemeralDiagnosticLogLock {
    EphemeralDiagnosticLogLock {
        path,
        token,
        file: Some(file),
    }
}

fn create_diagnostic_log_lock(path: &Path) -> Result<(String, fs::File), io::Error> {
    let token = diagnostic_log_lock_token();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    write_diagnostic_log_lock_token(path, &mut file, &token)?;
    Ok((token, file))
}

pub(crate) fn write_diagnostic_log_lock_token(
    path: &Path,
    writer: &mut impl Write,
    token: &str,
) -> Result<(), io::Error> {
    if let Err(err) = writer
        .write_all(token.as_bytes())
        .and_then(|()| writer.flush())
    {
        let _ = fs::remove_file(path);
        return Err(err);
    }
    Ok(())
}

fn diagnostic_log_lock_token() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    format!("{}:{}\n", std::process::id(), timestamp)
}

fn diagnostic_log_lock_is_stale(path: &Path) -> DiagnosticLogResult<bool> {
    reject_symlink(path)
        .map_err(|message| log_io_error("inspect", path, io::Error::other(message)))?;
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(log_io_error("stat", path, err)),
    };
    let modified = metadata
        .modified()
        .map_err(|err| log_io_error("stat", path, err))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    Ok(stale_diagnostic_log_lock_age(age))
}

pub(crate) fn stale_diagnostic_log_lock_age(age: Duration) -> bool {
    age >= Duration::from_secs(DIAGNOSTIC_LOG_LOCK_STALE_AFTER_SECS)
}
