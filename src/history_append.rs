use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::history::{read_history_records_from_path, HistoryCache};
use crate::history_compaction::{compact_history, should_compact_history};
use crate::logging::{render_check_log_record, DiagnosticLogError};
use crate::path_io_error::PathIoError;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;

#[cfg(test)]
pub(crate) fn append_history_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    let mut cache = HistoryCache::new();
    append_history_record_with_cache(root, expectation, record, &mut cache)
}

pub(crate) fn append_history_record_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    // The check pipeline exposes human-readable String errors, but this module
    // keeps I/O failures structured until the boundary so action, path, kind,
    // and source error stay tied together while the append is assembled.
    append_history_record_with_cache_inner(root, expectation, record, history_cache)
        .map_err(|err| err.to_string())
}

fn append_history_record_with_cache_inner(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
) -> Result<(), HistoryAppendError> {
    let path = history_cache.path(root, expectation)?;
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    let mut file = open_history_append_file(&path)?;
    let line = render_check_log_record(record)?;
    write_history_line(&mut file, &path, &line)?;
    flush_history_file(&mut file, &path)?;
    drop(file);
    history_cache.reusable_records.clear();
    let had_cached_records = history_cache.records.contains_key(&path);
    let should_compact = should_compact_history();
    // Once the line is flushed, the append has succeeded. Compaction and cache
    // refresh are maintenance steps, so failures there must not invite callers
    // to retry the append and duplicate the durable history record.
    let compacted = should_compact && compact_history(&path).is_ok();
    if had_cached_records {
        if compacted {
            match read_history_records_from_path(&path) {
                Ok(records) => {
                    history_cache.records.insert(path, records);
                }
                Err(_) => {
                    history_cache.records.remove(&path);
                }
            }
        } else if let Some(records) = history_cache.records.get_mut(&path) {
            records.push(record.clone());
        }
    }
    Ok(())
}

fn open_history_append_file(path: &Path) -> Result<fs::File, PathIoError> {
    reject_symlink(path)
        .map_err(|message| PathIoError::new("inspect", path, std::io::Error::other(message)))?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| PathIoError::new("open", path, source))
}

fn write_history_line(file: &mut fs::File, path: &Path, line: &str) -> Result<(), PathIoError> {
    file.write_all(line.as_bytes())
        .map_err(|source| PathIoError::new("write", path, source))
}

fn flush_history_file(file: &mut fs::File, path: &Path) -> Result<(), PathIoError> {
    file.flush()
        .map_err(|source| PathIoError::new("flush", path, source))
}

#[derive(Debug)]
enum HistoryAppendError {
    Message(String),
    Io(PathIoError),
}

impl fmt::Display for HistoryAppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryAppendError::Message(message) => formatter.write_str(message),
            HistoryAppendError::Io(err) => err.fmt(formatter),
        }
    }
}

impl Error for HistoryAppendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            HistoryAppendError::Message(_) => None,
            HistoryAppendError::Io(err) => Some(err),
        }
    }
}

impl From<String> for HistoryAppendError {
    fn from(message: String) -> HistoryAppendError {
        HistoryAppendError::Message(message)
    }
}

impl From<DiagnosticLogError> for HistoryAppendError {
    fn from(err: DiagnosticLogError) -> HistoryAppendError {
        HistoryAppendError::Message(err.to_string())
    }
}

impl From<PathIoError> for HistoryAppendError {
    fn from(err: PathIoError) -> HistoryAppendError {
        HistoryAppendError::Io(err)
    }
}
