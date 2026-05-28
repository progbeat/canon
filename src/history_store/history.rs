use crate::check_types::{contains_line_break, CheckRecord, CheckResult, SelectedExpectation};
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, reject_symlink,
    write_temp_file_then_replace,
};
use crate::git::resolve_git_path;
use crate::logging_error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::path_io_error::PathIoError;
use crate::time::parse_record_timestamp;
use crate::visible_tree_oid::{
    git_object_oid_has_known_shape, repository_native_object_oid_is_valid,
};
use crate::{
    CANON_CACHE_DIR_GIT_PATH, HISTORY_COMPACT_CHANCE_DENOMINATOR, HISTORY_COMPACT_KEEP_RECORDS,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub(crate) fn full_scope_reset_marker_file_name() -> &'static str {
    "full-scope-reset"
}

#[cfg(test)]
pub(crate) fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    read_history_records_from_path(&path)
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

pub(crate) fn parse_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<CheckRecord, String> {
    let record = serde_json::from_str::<CheckRecord>(line).map_err(|err| {
        format!(
            "invalid history JSON in {} line {}: {}",
            path.display(),
            line_number,
            err
        )
    })?;
    if !record_has_schema_valid_answer(&record) {
        return Err(format!(
            "invalid answer history record in {} line {}: records must be schema-valid responses with answer",
            path.display(),
            line_number
        ));
    }
    Ok(record)
}

fn record_has_schema_valid_answer(record: &CheckRecord) -> bool {
    validate_schema_valid_answer_history_record(record).is_ok()
}

fn validate_schema_valid_answer_history_record(record: &CheckRecord) -> Result<(), String> {
    // `serde_json::<CheckRecord>` has already required the Cache spec prefix
    // fields with no defaults: observed, evidence, qScope, and visibleTreeOid.
    // The history layer enforces the parts that are not expressible by the
    // struct shape: answer/error one-of, single-line answer text, UTC
    // timestamp, and Git object-ID syntax for visibleTreeOid.
    if record.error.is_some() {
        return Err("error responses are not answer history records".to_string());
    }
    if record.observed.trim().is_empty() || contains_line_break(&record.observed) {
        return Err("observed answer must be a non-empty single-line string".to_string());
    }
    if parse_record_timestamp(&record.timestamp).is_none() {
        return Err("timestamp must be UTC in YYYY-MM-DDTHH:MM:SSZ form".to_string());
    }
    if !git_object_oid_has_known_shape(&record.visible_tree_oid) {
        return Err("visibleTreeOid must be a Git object ID hex string".to_string());
    }
    Ok(())
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
        let records = read_history_records_from_path(&path)?;
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

pub(crate) fn full_scope_reset_marker_path_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<PathBuf, String> {
    Ok(history_cache
        .path(root, expectation)?
        .with_file_name(full_scope_reset_marker_file_name()))
}

pub(crate) fn write_full_scope_reset_marker_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    let path = full_scope_reset_marker_path_with_cache(root, expectation, history_cache)?;
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    let temp_path = compact_history_temp_path(&path)?;
    write_temp_file_then_replace(&temp_path, &path, |file| {
        file.write_all(b"full\n")
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
    })
}

pub(crate) fn full_scope_reset_marker_exists_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<bool, String> {
    let path = full_scope_reset_marker_path_with_cache(root, expectation, history_cache)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "refusing to use symlinked full-scope reset marker {}",
                    path.display()
                ));
            }
            Ok(true)
        }
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

pub(crate) fn remove_full_scope_reset_marker_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    let path = full_scope_reset_marker_path_with_cache(root, expectation, history_cache)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to delete {}: {}", path.display(), err)),
    }
}

pub(crate) fn render_answer_history_record(record: &CheckRecord) -> DiagnosticLogResult<String> {
    validate_schema_valid_answer_history_record(record)
        .map_err(|message| external_log_error("render answer history record", message))?;
    // History records intentionally start with the Cache spec's required
    // "at least" answer-history field prefix. The spec is not a closed schema:
    // extra persisted metadata is allowed as long as it follows that prefix.
    // Expectation references use the resolved full ID, never the
    // display/selector prefix.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        observed: &record.observed,
        evidence: &record.evidence,
        q_scope: &record.scope,
        visible_tree_oid: &record.visible_tree_oid,
        result: record.result,
        id: &record.id,
        prompt: record.prompt_text(),
        expected: record.expected_text(),
        cache_key: record.cache_key.as_deref(),
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
    // Optional cache/debug metadata. These fields are deliberately after the
    // required prefix so they do not change the answer-history format promised
    // by the Cache spec.
    result: CheckResult,
    id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<&'a str>,
    #[serde(rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    cache_key: Option<&'a str>,
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
    let _ = remove_full_scope_reset_marker_with_cache(root, expectation, history_cache);
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

pub(crate) fn compact_history(path: &Path) -> Result<(), String> {
    let mut valid_lines = 0usize;
    let mut invalid_lines = 0usize;
    let mut lines = std::collections::VecDeque::new();
    for_each_nonempty_line(path, |line_number, line| {
        if valid_history_record_line(path, line_number, &line) {
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

fn valid_history_record_line(path: &Path, line_number: usize, line: &str) -> bool {
    parse_history_record_line(path, line_number, line).is_ok()
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
