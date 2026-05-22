use crate::fs_util::reject_symlink;
use crate::logging_config::{active_log_max_bytes, diagnostic_log_files};
use crate::logging_error::{
    log_io_error, log_rename_error, DiagnosticLogError, DiagnosticLogResult,
};
use crate::logging_fs::remove_file_if_exists;
use crate::DiagnosticLogConfig;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub(crate) fn open_runtime_log_file(path: &Path) -> DiagnosticLogResult<fs::File> {
    reject_symlink(path)
        .map_err(|message| log_io_error("inspect", path, io::Error::other(message)))?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| log_io_error("open", path, err))
}

pub(crate) fn append_runtime_log_event_to_file(
    path: &Path,
    file: &mut fs::File,
    line: &str,
) -> DiagnosticLogResult<()> {
    file.write_all(line.as_bytes())
        .map_err(|err| log_io_error("write", path, err))?;
    file.flush().map_err(|err| log_io_error("flush", path, err))
}

pub(crate) fn rotate_diagnostic_logs_with_config(
    log_dir: &Path,
    config: &DiagnosticLogConfig,
) -> DiagnosticLogResult<()> {
    if config.max_bytes == 0 {
        return Ok(());
    }
    let files = diagnostic_log_files(config)?;
    let active = log_dir.join(files[0]);
    let active_limit = active_log_max_bytes(config, files.len());
    let should_rotate = match active.metadata() {
        Ok(metadata) => metadata.len() > active_limit,
        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
        Err(err) => return Err(log_io_error("stat", &active, err)),
    };
    if should_rotate {
        rotate_active_diagnostic_logs(log_dir, files)?;
    }
    prune_diagnostic_logs_to_limit(log_dir, config)?;
    Ok(())
}

pub(crate) fn active_log_size(path: &Path) -> DiagnosticLogResult<u64> {
    reject_symlink(path)
        .map_err(|message| log_io_error("inspect", path, io::Error::other(message)))?;
    match path.metadata() {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(log_io_error("stat", path, err)),
    }
}

pub(crate) fn rotate_active_diagnostic_logs(
    log_dir: &Path,
    files: &[&str],
) -> DiagnosticLogResult<()> {
    let oldest = log_dir.join(files[files.len() - 1]);
    remove_file_if_exists(&oldest)?;
    for index in (0..files.len() - 1).rev() {
        let from = log_dir.join(files[index]);
        let to = log_dir.join(files[index + 1]);
        rename_file_if_exists(&from, &to)?;
    }
    Ok(())
}

pub(crate) fn prune_diagnostic_logs_to_limit(
    log_dir: &Path,
    config: &DiagnosticLogConfig,
) -> DiagnosticLogResult<()> {
    if config.max_bytes == 0 {
        return Ok(());
    }
    let files = diagnostic_log_files(config)?;
    for index in (1..files.len()).rev() {
        if diagnostic_log_dir_size(log_dir, config)? <= config.max_bytes {
            return Ok(());
        }
        let path = log_dir.join(files[index]);
        remove_file_if_exists(&path)?;
    }
    Ok(())
}

fn rename_file_if_exists(from: &Path, to: &Path) -> DiagnosticLogResult<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(log_rename_error(from, to, err)),
    }
}

fn diagnostic_log_dir_size(
    log_dir: &Path,
    config: &DiagnosticLogConfig,
) -> DiagnosticLogResult<u64> {
    let mut total = 0u64;
    for file_name in diagnostic_log_files(config)? {
        let path = log_dir.join(file_name);
        reject_symlink(&path)
            .map_err(|message| log_io_error("inspect", &path, io::Error::other(message)))?;
        let size = match path.metadata() {
            Ok(metadata) => metadata.len(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(log_io_error("stat", &path, err)),
        };
        total = total
            .checked_add(size)
            .ok_or_else(|| DiagnosticLogError::SizeOverflow {
                path: log_dir.to_path_buf(),
            })?;
    }
    Ok(total)
}
