use crate::check_types::{CheckRecord, CheckResult, EvaluatorResponseJson, SelectedExpectation};
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, reject_symlink,
    write_temp_file_then_replace,
};
use crate::git::resolve_git_path;
use crate::logging_error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::path_io_error::PathIoError;
use crate::time::parse_record_timestamp;
use crate::visible_tree_oid::{
    git_object_oid_has_hex_len, git_object_oid_has_known_shape,
    repository_native_object_oid_hex_len, repository_native_object_oid_is_valid,
    VisibleTreeOidCache,
};
use crate::{
    CANON_CACHE_DIR_GIT_PATH, HISTORY_COMPACT_CHANCE_DENOMINATOR, HISTORY_COMPACT_KEEP_RECORDS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Cache-spec answer history storage is owned end-to-end in this file: path
// resolution, JSONL parsing, answer-only append, required field-order
// rendering, and probabilistic compaction. Runtime `CheckRecord` construction
// computes the actual `visibleTreeOid` before append in
// `check_interrogation_records::finalize_parsed_answer`, using
// `VisibleTreeOidCache::staged_visible_tree_oid` for the enforced q-scope; this
// layer preserves that native Git tree OID instead of deriving a second
// fingerprint while writing JSONL. `history_append.rs`,
// `history_compaction.rs`, and `logging::render_answer_history_record` are thin
// import-compatibility wrappers around these functions.

static HISTORY_COMPACT_CHANCE_COUNTER: AtomicU64 = AtomicU64::new(0);
static HISTORY_COMPACT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HISTORY_LOCK_RETRY_COUNT: usize = 100;
const HISTORY_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(10);
const HISTORY_LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

#[cfg(test)]
pub(crate) fn history_path(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<PathBuf, String> {
    Ok(resolve_git_path(root, CANON_CACHE_DIR_GIT_PATH)?
        .join(&expectation.id)
        .join(history_file_name()))
}

pub(crate) fn history_file_name() -> &'static str {
    "history.jsonl"
}

#[cfg(test)]
pub(crate) fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    read_repository_history_records_from_path(root, &path)
}

pub(crate) fn read_history_records_from_path(path: &Path) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        match parse_history_record_line(path, line_number, &line) {
            Ok(record) => records.push(record),
            Err(_) => {
                // History is a reusable cache, not authoritative project data.
                // Corrupt cache lines are ignored here and dropped by the same
                // parser during compaction, while real file I/O errors still
                // propagate from `for_each_nonempty_line`.
            }
        }
        Ok(())
    })?;
    Ok(records)
}

pub(crate) fn read_repository_history_records_from_path(
    root: &Path,
    path: &Path,
) -> Result<Vec<CheckRecord>, String> {
    let native_oid_hex_len = repository_native_object_oid_hex_len(root)?;
    let mut records = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        match parse_history_record_line(path, line_number, &line) {
            Ok(record) => {
                if git_object_oid_has_hex_len(&record.visible_tree_oid, native_oid_hex_len) {
                    records.push(record);
                } else {
                    // The Cache spec defines visibleTreeOid as a repository-native
                    // Git object ID. A valid-looking SHA-1 in a SHA-256 repo, or
                    // vice versa, is corrupt cache data and must not be reused.
                }
            }
            Err(_) => {
                // History is a reusable cache, not authoritative project data.
                // Corrupt cache lines are ignored here and dropped by the same
                // parser during compaction, while real file I/O errors still
                // propagate from `for_each_nonempty_line`.
            }
        }
        Ok(())
    })?;
    Ok(records)
}

pub(crate) fn parse_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<CheckRecord, String> {
    let record = serde_json::from_str::<HistoryReadRecord>(line).map_err(|err| {
        format!(
            "invalid history JSON in {} line {}: {}",
            path.display(),
            line_number,
            err
        )
    })?;
    let record = record.into_check_record();
    validate_schema_valid_answer_history_record(&record).map_err(|message| {
        format!(
            "invalid answer history record in {} line {}: records must be schema-valid responses with answer: {}",
            path.display(),
            line_number,
            message
        )
    })?;
    Ok(record)
}

#[derive(Deserialize)]
struct HistoryReadRecord {
    timestamp: String,
    observed: String,
    evidence: String,
    #[serde(rename = "qScope")]
    q_scope: Vec<String>,
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: String,
    #[serde(default)]
    #[serde(flatten)]
    extra_fields: BTreeMap<String, Value>,
}

impl HistoryReadRecord {
    fn into_check_record(self) -> CheckRecord {
        let has_error_field = self.extra_fields.contains_key("error");
        CheckRecord {
            timestamp: self.timestamp,
            number: 0,
            result: CheckResult::Fail,
            prompt: None,
            expected: None,
            observed: self.observed,
            error: has_error_field.then(|| "error".to_string()),
            evidence: self.evidence,
            scope: self.q_scope,
            suggested_q_scope: None,
            visible_tree_oid: self.visible_tree_oid,
            id: String::new(),
            display_id: String::new(),
            cache_key: None,
        }
    }
}

fn validate_schema_valid_answer_history_record(record: &CheckRecord) -> Result<(), String> {
    // `HistoryReadRecord` requires the Cache spec prefix fields with no
    // defaults, while allowing extra metadata because the spec says "at least"
    // those fields. Cache records store the evaluator response's `answer` value
    // as `observed`, so reconstruct a minimal answer response and validate it
    // with the same evaluator response schema used at runtime.
    if record.error.is_some() {
        return Err("error responses are not answer history records".to_string());
    }
    validate_history_answer_response_schema(record)?;
    if parse_record_timestamp(&record.timestamp).is_none() {
        return Err("timestamp must be UTC in YYYY-MM-DDTHH:MM:SSZ form".to_string());
    }
    if !git_object_oid_has_known_shape(&record.visible_tree_oid) {
        return Err("visibleTreeOid must be a Git object ID hex string".to_string());
    }
    Ok(())
}

fn validate_history_answer_response_schema(record: &CheckRecord) -> Result<(), String> {
    let response = EvaluatorResponseJson {
        answer: Some(record.observed.clone()),
        error: None,
        evidence: record.evidence.clone(),
        q_scope_suggestion: None,
    };
    response.validate_schema().map_err(|message| {
        format!("observed must match evaluator response answer schema: {message}")
    })
}

fn validate_appendable_answer_history_record(
    root: &Path,
    record: &CheckRecord,
) -> Result<(), String> {
    // Append-time validation checks that a runtime-produced record is a valid
    // answer-history row and that its `visibleTreeOid` uses the repository's
    // native object format. It intentionally does not recompute the q-scope's
    // current visible tree here: history rows are later read when their stored
    // qScope may describe an older Git state, and cache reuse is the layer that
    // compares stored OIDs with freshly computed current OIDs.
    validate_schema_valid_answer_history_record(record)?;
    if !repository_native_object_oid_is_valid(root, &record.visible_tree_oid)? {
        return Err(
            "visibleTreeOid must match this repository's Git object hash algorithm".to_string(),
        );
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct HistoryCache {
    pub(crate) cache_dirs: BTreeMap<PathBuf, PathBuf>,
    pub(crate) paths: BTreeMap<(PathBuf, String), PathBuf>,
    pub(crate) records: BTreeMap<PathBuf, Vec<CheckRecord>>,
    pub(crate) latest_non_pass: BTreeMap<PathBuf, Option<u64>>,
}

impl HistoryCache {
    pub(crate) fn new() -> HistoryCache {
        HistoryCache::default()
    }

    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        if let Some(records) = self.records.get(&path) {
            return Ok(records.clone());
        }
        // Runtime cache reads know the repository root, so this is where
        // answer-history rows are checked against the repository-native Git
        // object hash algorithm. The lower-level line parser only validates the
        // portable JSONL shape used by compaction and parser tests.
        let records = read_repository_history_records_from_path(root, &path)?;
        self.records.insert(path, records.clone());
        Ok(records)
    }

    pub(crate) fn path(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.paths.get(&key) {
            return Ok(path.clone());
        }
        let path = self
            .cache_dir(root)?
            .join(&expectation.id)
            .join(history_file_name());
        self.paths.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn cache_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.cache_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(root, CANON_CACHE_DIR_GIT_PATH)?;
        self.cache_dirs.insert(key, path.clone());
        Ok(path)
    }
}

pub(crate) fn render_answer_history_record(record: &CheckRecord) -> DiagnosticLogResult<String> {
    validate_schema_valid_answer_history_record(record)
        .map_err(|message| external_log_error("render answer history record", message))?;
    // Keep answer-history rows to the Cache spec fields. Current result is
    // derived from observed vs the current expectation rather than persisted.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        observed: &record.observed,
        evidence: &record.evidence,
        q_scope: &record.scope,
        visible_tree_oid: &record.visible_tree_oid,
    };
    answer_history_json_line(&history)
}

fn answer_history_json_line(value: &impl Serialize) -> DiagnosticLogResult<String> {
    let mut output = serde_json::to_string(value).map_err(|source| DiagnosticLogError::Json {
        description: "history log record",
        source,
    })?;
    output.push('\n');
    Ok(output)
}

#[derive(Serialize)]
struct HistoryLogRecord<'a> {
    // Required Cache spec prefix. Keep these fields first and in this order:
    // timestamp, observed, evidence, qScope, visibleTreeOid.
    timestamp: &'a str,
    observed: &'a str,
    evidence: &'a str,
    #[serde(rename = "qScope")]
    q_scope: &'a [String],
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: &'a str,
}

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

pub(crate) fn append_current_history_record_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<(), String> {
    validate_current_visible_tree_oid(root, expectation, record, visible_tree_oid_cache)?;
    append_history_record_with_cache(root, expectation, record, history_cache)
}

fn validate_current_visible_tree_oid(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<(), String> {
    let current_visible_tree_oid =
        visible_tree_oid_cache.staged_visible_tree_oid(root, &expectation.agent, &record.scope)?;
    if record.visible_tree_oid != current_visible_tree_oid {
        return Err(
            "visibleTreeOid must match the current repository visible tree for qScope".to_string(),
        );
    }
    Ok(())
}

fn append_history_record_with_cache_inner(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
) -> Result<(), HistoryAppendError> {
    // Cache spec answer history is JSON Lines containing only schema-valid
    // evaluator responses with `answer`. `render_answer_history_record` writes
    // the required field prefix in order: timestamp, observed, evidence,
    // qScope, visibleTreeOid. `should_compact_history` implements the
    // approximate 1-in-16 trigger, and `compact_history` retains the latest 8
    // valid JSON object records.
    validate_appendable_answer_history_record(root, record).map_err(|message| {
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
    let line = render_answer_history_record(record)?;
    write_history_line(&mut file, &path, &line)?;
    flush_history_file(&mut file, &path)?;
    drop(file);
    let had_cached_records = history_cache.records.contains_key(&path);
    let should_compact = should_compact_history();
    // Once the line is flushed, the append has succeeded. Compaction and cache
    // refresh are maintenance steps, so failures there must not invite callers
    // to retry the append and duplicate the durable history record.
    let compacted = should_compact && compact_repository_history_locked(root, &path).is_ok();
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

pub(crate) fn should_compact_history() -> bool {
    should_compact_history_for_seed(compaction_chance_seed())
}

pub(crate) fn should_compact_history_for_seed(seed: u64) -> bool {
    seed.is_multiple_of(HISTORY_COMPACT_CHANCE_DENOMINATOR)
}

fn compaction_chance_seed() -> u64 {
    let counter = HISTORY_COMPACT_CHANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ counter.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ process::id() as u64
}

#[cfg(test)]
pub(crate) fn compact_history(path: &Path) -> Result<(), String> {
    let _lock = lock_history_file(path).map_err(|err| err.to_string())?;
    compact_history_locked_with_native_oid_len(path, None)
}

#[cfg(test)]
pub(crate) fn compact_repository_history(root: &Path, path: &Path) -> Result<(), String> {
    let _lock = lock_history_file(path).map_err(|err| err.to_string())?;
    compact_repository_history_locked(root, path)
}

fn compact_repository_history_locked(root: &Path, path: &Path) -> Result<(), String> {
    let native_oid_hex_len = repository_native_object_oid_hex_len(root)?;
    compact_history_locked_with_native_oid_len(path, Some(native_oid_hex_len))
}

fn compact_history_locked_with_native_oid_len(
    path: &Path,
    native_oid_hex_len: Option<usize>,
) -> Result<(), String> {
    let mut valid_lines = 0usize;
    let mut invalid_lines = 0usize;
    let mut lines = std::collections::VecDeque::new();
    for_each_nonempty_line(path, |line_number, line| {
        if valid_history_record_line(path, line_number, &line, native_oid_hex_len) {
            valid_lines += 1;
            lines.push_back(line);
            if lines.len() > HISTORY_COMPACT_KEEP_RECORDS {
                lines.pop_front();
            }
        } else {
            invalid_lines += 1;
        }
        Ok(())
    })?;
    if valid_lines <= HISTORY_COMPACT_KEEP_RECORDS && invalid_lines == 0 {
        return Ok(());
    }
    let temp_path = compact_history_temp_path(path)?;
    write_temp_file_then_replace(&temp_path, path, |file| {
        for line in lines {
            file.write_all(line.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            file.write_all(b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })
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
    if let Some(parent) = lock_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    for _ in 0..HISTORY_LOCK_RETRY_COUNT {
        match create_history_lock(&lock_path) {
            Ok(()) => return Ok(HistoryFileLock { path: lock_path }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if history_lock_is_stale(&lock_path)? {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                thread::sleep(HISTORY_LOCK_RETRY_SLEEP);
            }
            Err(err) => {
                return Err(HistoryAppendError::Message(format!(
                    "failed to lock {}: {}",
                    lock_path.display(),
                    err
                )));
            }
        }
    }
    Err(HistoryAppendError::Message(format!(
        "failed to lock {}: lock is already held",
        lock_path.display()
    )))
}

fn create_history_lock(path: &Path) -> Result<(), std::io::Error> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    writeln!(file, "{}", process::id())?;
    file.flush()
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

fn valid_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
    native_oid_hex_len: Option<usize>,
) -> bool {
    let Ok(record) = parse_history_record_line(path, line_number, line) else {
        return false;
    };
    native_oid_hex_len
        .is_none_or(|hex_len| git_object_oid_has_hex_len(&record.visible_tree_oid, hex_len))
}

pub(crate) fn compact_history_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("history path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = HISTORY_COMPACT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
