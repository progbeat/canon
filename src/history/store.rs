use crate::check::{CheckRecord, CheckResult, EvaluatorResponseJson, SelectedExpectation};
use crate::fs_util::{
    ensure_dir_without_symlinks, for_each_nonempty_line, reject_symlink,
    write_temp_file_then_replace,
};
use crate::git::resolve_git_path;
use crate::git::{
    git_object_oid_has_hex_len, git_object_oid_has_known_shape,
    repository_native_object_oid_hex_len, TreeSource, VisibleTreeOidCache,
};
use crate::logs::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::path_io_error::PathIoError;
use crate::scope::visible_scope;
use crate::state_paths::CANON_CACHE_DIR_GIT_PATH;
use crate::time::parse_record_timestamp;
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
// `check::interrogation::records`, using `VisibleTreeOidCache` for the enforced
// scope; this layer preserves that native Git tree OID instead of deriving a
// second fingerprint while writing JSONL.

static HISTORY_COMPACT_CHANCE_COUNTER: AtomicU64 = AtomicU64::new(0);
static HISTORY_COMPACT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HISTORY_COMPACT_KEEP_RECORDS: usize = 8;
const HISTORY_COMPACT_CHANCE_DENOMINATOR: u64 = 16;
const HISTORY_LOCK_RETRY_COUNT: usize = 100;
const HISTORY_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(10);
const HISTORY_LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

pub(crate) fn history_file_name() -> &'static str {
    "history.jsonl"
}

pub(crate) fn read_repository_history_records_from_path(
    root: &Path,
    path: &Path,
    expected_answer: &str,
) -> Result<Vec<CheckRecord>, String> {
    let native_oid_hex_len = repository_native_object_oid_hex_len(root)?;
    let mut records = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        match parse_history_record_line_for_expected(
            path,
            line_number,
            &line,
            Some(expected_answer),
        ) {
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
    // Shape-only cache maintenance parse. Without a current expectation there
    // is no current expected answer to compare against, so schema-valid answer
    // rows parse as neutral passes. Runtime history reads must use
    // `read_repository_history_records_from_path`, which derives `result` from
    // the current expectation's expected answer.
    parse_history_record_line_for_expected(path, line_number, line, None)
}

fn parse_history_record_line_for_expected(
    path: &Path,
    line_number: usize,
    line: &str,
    expected_answer: Option<&str>,
) -> Result<CheckRecord, String> {
    let record = serde_json::from_str::<HistoryReadRecord>(line).map_err(|err| {
        format!(
            "invalid history JSON in {} line {}: {}",
            path.display(),
            line_number,
            err
        )
    })?;
    let record = record.into_check_record(expected_answer);
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
    #[serde(rename = "visibleScope", alias = "qScope", alias = "scope")]
    visible_scope: Vec<String>,
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: String,
    #[serde(default)]
    #[serde(flatten)]
    extra_fields: BTreeMap<String, Value>,
}

impl HistoryReadRecord {
    fn into_check_record(self, expected_answer: Option<&str>) -> CheckRecord {
        let has_error_field = self.extra_fields.contains_key("error");
        let result = expected_answer
            .map(|expected| CheckResult::from_expected_answer(expected, &self.observed))
            .unwrap_or(CheckResult::Pass);
        CheckRecord {
            timestamp: self.timestamp,
            number: 0,
            result,
            prompt: None,
            expected: expected_answer.map(str::to_string),
            observed: self.observed,
            error: has_error_field.then(|| "error".to_string()),
            evidence: self.evidence,
            scope: self.visible_scope,
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
        // History rows store the cache-required answer fields, not the full
        // evaluator response. Use a schema-valid placeholder so this check
        // continues to validate the persisted answer/evidence contract.
        q_scope_suggestion: vec![".".to_string()],
    };
    response.validate_schema().map_err(|message| {
        format!("observed must match evaluator response answer schema: {message}")
    })
}

fn validate_appendable_answer_history_record(
    record: &CheckRecord,
    native_oid_hex_len: usize,
) -> Result<(), String> {
    // Append-time validation checks that a runtime-produced record is a valid
    // answer-history row and that its `visibleTreeOid` uses the repository's
    // native object format. It intentionally does not recompute the stored
    // visible scope's current visible tree here: history rows are later read
    // when their stored visibleScope may describe an older Git state, and cache
    // reuse is the layer that compares stored OIDs with freshly computed
    // current OIDs.
    validate_schema_valid_answer_history_record(record)?;
    if !git_object_oid_has_hex_len(&record.visible_tree_oid, native_oid_hex_len) {
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
    pub(crate) records: BTreeMap<HistoryRecordsKey, Vec<CheckRecord>>,
    // `check::order_state` owns the latest-non-pass marker policy; the cache
    // lives here with the other history path/read caches for one run.
    pub(crate) latest_non_pass: BTreeMap<PathBuf, Option<u64>>,
}

type HistoryRecordsKey = (PathBuf, String);

impl HistoryCache {
    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        let records_key = history_records_key(&path, &expectation.a);
        if let Some(records) = self.records.get(&records_key) {
            return Ok(records.clone());
        }
        // Runtime cache reads know the repository root, so this is where
        // answer-history rows are checked against the repository-native Git
        // object hash algorithm. The lower-level line parser only validates the
        // portable JSONL shape used by compaction and parser tests.
        let records = read_repository_history_records_from_path(root, &path, &expectation.a)?;
        self.records.insert(records_key, records.clone());
        Ok(records)
    }

    fn record_keys_for_path(&self, path: &Path) -> Vec<HistoryRecordsKey> {
        self.records
            .keys()
            .filter(|(cached_path, _)| cached_path == path)
            .cloned()
            .collect()
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

pub(crate) fn render_answer_history_record(
    agent: &crate::config_types::AgentConfig,
    record: &CheckRecord,
) -> DiagnosticLogResult<String> {
    validate_schema_valid_answer_history_record(record)
        .map_err(|message| external_log_error("render answer history record", message))?;
    // Keep answer-history rows to the Cache spec fields. Current result is
    // derived from observed vs the current expectation rather than persisted.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        observed: &record.observed,
        evidence: &record.evidence,
        visible_scope: &visible_scope(agent, &record.scope)
            .map_err(|message| external_log_error("render answer history record", message))?,
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
    // timestamp, observed, evidence, visibleScope, visibleTreeOid.
    timestamp: &'a str,
    observed: &'a str,
    evidence: &'a str,
    #[serde(rename = "visibleScope")]
    visible_scope: &'a [String],
    #[serde(rename = "visibleTreeOid")]
    visible_tree_oid: &'a str,
}

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
    // visibleScope, visibleTreeOid. `should_compact_history` implements the
    // approximate 1-in-16 trigger, and `compact_history` retains the latest 8
    // valid JSON object records.
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

fn history_records_key(path: &Path, expected_answer: &str) -> HistoryRecordsKey {
    (path.to_path_buf(), expected_answer.to_string())
}

fn record_for_expected_answer(record: &CheckRecord, expected_answer: &str) -> CheckRecord {
    let mut record = record.clone();
    record.result = CheckResult::from_expected_answer(expected_answer, &record.observed);
    record.expected = Some(expected_answer.to_string());
    record
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

#[cfg(test)]
mod tests {
    use super::render_answer_history_record;
    use crate::check::{CheckRecord, CheckResult};
    use crate::config_types::AgentConfig;
    use serde_json::Value;

    #[test]
    fn answer_history_record_writes_visible_scope_field() {
        let agent = AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: vec!["target/**".to_string()],
            plugins: Vec::new(),
        };
        let record = CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: Some("Does it pass?".to_string()),
            expected: Some("yes".to_string()),
            observed: "yes".to_string(),
            error: None,
            evidence: "`src/main.rs`".to_string(),
            scope: vec![".".to_string()],
            suggested_q_scope: None,
            visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
            cache_key: None,
        };

        let line = render_answer_history_record(&agent, &record).unwrap();
        let json: Value = serde_json::from_str(&line).unwrap();

        assert!(json.get("qScope").is_none());
        assert_eq!(
            json.get("visibleScope"),
            Some(&serde_json::json!([".", ":(exclude,glob)target/**"]))
        );
    }
}
