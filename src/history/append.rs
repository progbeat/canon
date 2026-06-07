use crate::check::{CheckRecord, SelectedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::compact::{compact_repository_history_locked, should_compact_history};
use crate::history::record::{
    read_repository_history_records_from_path, render_answer_history_record,
    validate_appendable_answer_history_record,
};
use crate::history::store::{record_for_expected_answer, HistoryCache};
use crate::logs::DiagnosticLogError;
use crate::path_io_error::PathIoError;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const HISTORY_LOCK_RETRY_COUNT: usize = 100;
const HISTORY_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(10);
const HISTORY_LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

pub(crate) fn append_current_history_record_with_cache(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<(), String> {
    validate_current_visible_tree_oid(root, source, expectation, record, visible_tree_oid_cache)?;
    let native_oid_hex_len = visible_tree_oid_cache.repository_native_object_oid_hex_len(root)?;
    append_history_record_with_cache_inner(
        root,
        expectation,
        record,
        history_cache,
        native_oid_hex_len,
    )
    .map_err(|err| err.to_string())
}

fn validate_current_visible_tree_oid(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<(), String> {
    let current_visible_tree_oid =
        visible_tree_oid_cache.visible_tree_oid(root, source, &expectation.agent, &record.scope)?;
    if record.visible_tree_oid != current_visible_tree_oid {
        return Err(
            "visibleTreeOid must match the current repository visible tree for visibleScope"
                .to_string(),
        );
    }
    Ok(())
}

fn append_history_record_with_cache_inner(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
    native_oid_hex_len: usize,
) -> Result<(), HistoryAppendError> {
    // Cache spec answer history is JSON Lines containing only schema-valid
    // evaluator responses with `answer`. `render_answer_history_record` writes
    // the required field prefix in order: timestamp, observed, evidence,
    // visibleScope, visibleTreeOid. Compaction keeps the latest valid records.
    validate_appendable_answer_history_record(record, native_oid_hex_len).map_err(|message| {
        HistoryAppendError::Message(format!(
            "answer history records must be schema-valid responses with answer: {message}"
        ))
    })?;
    let path = history_cache.path(root, expectation)?;
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    let _lock = lock_history_file(&path)?;
    let mut file = open_history_append_file(&path)?;
    let line = render_answer_history_record(&expectation.agent, record)?;
    write_history_line(&mut file, &path, &line)?;
    flush_history_file(&mut file, &path)?;
    drop(file);
    let cached_record_keys = history_cache.record_keys_for_path(&path);
    let should_compact = should_compact_history();
    // Once the line is flushed, the append has succeeded. Compaction and cache
    // refresh are maintenance steps, so failures there must not invite callers
    // to retry the append and duplicate the durable history record.
    let compacted = should_compact && compact_repository_history_locked(root, &path).is_ok();
    if !cached_record_keys.is_empty() {
        if compacted {
            for records_key in cached_record_keys {
                match read_repository_history_records_from_path(root, &path, &records_key.1) {
                    Ok(records) => {
                        history_cache.records.insert(records_key, records);
                    }
                    Err(_) => {
                        history_cache.records.remove(&records_key);
                    }
                }
            }
        } else {
            for records_key in cached_record_keys {
                if let Some(records) = history_cache.records.get_mut(&records_key) {
                    records.push(record_for_expected_answer(record, &records_key.1));
                }
            }
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
            HistoryAppendError::Io(err) => write!(formatter, "{err}"),
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

struct HistoryFileLock {
    path: PathBuf,
}

impl Drop for HistoryFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_history_file(path: &Path) -> Result<HistoryFileLock, HistoryAppendError> {
    let lock_path = history_lock_path(path)?;
    for _ in 0..HISTORY_LOCK_RETRY_COUNT {
        match create_history_lock(&lock_path) {
            Ok(()) => return Ok(HistoryFileLock { path: lock_path }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if history_lock_is_stale(&lock_path)? {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                std::thread::sleep(HISTORY_LOCK_RETRY_SLEEP);
            }
            Err(err) => {
                return Err(HistoryAppendError::Message(format!(
                    "failed to create history lock {}: {}",
                    lock_path.display(),
                    err
                )));
            }
        }
    }
    Err(HistoryAppendError::Message(format!(
        "timed out waiting for history lock {}",
        lock_path.display()
    )))
}

fn create_history_lock(path: &Path) -> Result<(), std::io::Error> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
}

fn history_lock_is_stale(path: &Path) -> Result<bool, HistoryAppendError> {
    let metadata = fs::metadata(path).map_err(|err| {
        HistoryAppendError::Message(format!("failed to inspect {}: {}", path.display(), err))
    })?;
    let modified = metadata.modified().map_err(|err| {
        HistoryAppendError::Message(format!("failed to inspect {}: {}", path.display(), err))
    })?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    Ok(age >= HISTORY_LOCK_STALE_AFTER)
}

fn history_lock_path(path: &Path) -> Result<PathBuf, HistoryAppendError> {
    let file_name = path.file_name().ok_or_else(|| {
        HistoryAppendError::Message(format!("history path has no file name: {}", path.display()))
    })?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    Ok(path.with_file_name(lock_name))
}
