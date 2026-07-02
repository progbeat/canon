use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink, write_temp_file_then_replace};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::hash_60;
use crate::scope::visible_scope;
use crate::time::{format_record_timestamp, parse_record_timestamp, unix_timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use super::XpecStateCache;

static LAST_RESULT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LastResultStatus {
    Pass,
    Fail,
    Error,
}

impl LastResultStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "pass",
            LastResultStatus::Fail => "fail",
            LastResultStatus::Error => "error",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "last-pass.json",
            LastResultStatus::Fail => "last-fail.json",
            LastResultStatus::Error => "last-error.json",
        }
    }

    fn check_result(self) -> CheckResult {
        match self {
            LastResultStatus::Pass => CheckResult::Pass,
            LastResultStatus::Fail | LastResultStatus::Error => CheckResult::Fail,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LastResult {
    // This struct is the persisted last-result schema. Prompt-rendering inputs
    // such as `diff-from` are intentionally not part of this state record. The
    // containing xpec directory is keyed by the full expectation ID; the JSON
    // body does not persist the expectation ID or human display prefix.
    #[serde(rename = "responseTimestamp")]
    pub(crate) response_timestamp: String,
    #[serde(rename = "updatedTimestamp")]
    pub(crate) updated_timestamp: String,
    pub(crate) status: LastResultStatus,
    pub(crate) response: Value,
    #[serde(rename = "qScope")]
    pub(crate) q_scope: Vec<String>,
    #[serde(rename = "visibleScope")]
    pub(crate) visible_scope: Vec<String>,
    #[serde(
        rename = "checkedTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) checked_tree_oid: Option<String>,
    #[serde(
        rename = "visibleTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) visible_tree_oid: Option<String>,
}

impl LastResult {
    pub(crate) fn answer(&self) -> Option<&str> {
        self.response.get("answer").and_then(Value::as_str)
    }

    fn error(&self) -> Option<&str> {
        self.response.get("error").and_then(Value::as_str)
    }

    fn evidence(&self) -> String {
        self.response
            .get("evidence")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn question_scope_suggestion(&self) -> Option<Vec<String>> {
        // Last-result `response` is the normalized evaluator response; the
        // applied q-scope is stored separately in `qScope`.
        self.response
            .get("qScopeSuggestion")?
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_string))
            .collect()
    }
}

impl XpecStateCache {
    pub(crate) fn read_last_pass_q_scope(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<Vec<String>>, String> {
        Ok(self
            .read_last_pass(root, expectation)?
            .map(|result| result.q_scope))
    }

    pub(crate) fn read_last_pass(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Pass)
    }

    pub(crate) fn read_last_fail(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Fail)
    }

    pub(crate) fn read_last_error(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Error)
    }

    pub(crate) fn read_last_result(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
    ) -> Result<Option<LastResult>, String> {
        let key = (root.to_path_buf(), expectation.id.clone(), status);
        if let Some(cached) = self.last_results.get(&key) {
            return Ok(cached.clone());
        }
        let path = self.last_result_path(root, expectation, status)?;
        let result = read_last_result_path(&path, status)?;
        self.last_results.insert(key, result.clone());
        Ok(result)
    }

    pub(crate) fn read_same_tree_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
    ) -> Result<Vec<LastResult>, String> {
        assert!(matches!(
            status,
            LastResultStatus::Pass | LastResultStatus::Fail
        ));
        let key = (root.to_path_buf(), expectation.id.clone(), status);
        if let Some(cached) = self.same_tree_records.get(&key) {
            return Ok(cached.clone());
        }
        let results = read_same_tree_records_dir(
            &self.same_tree_records_dir(root, expectation, status)?,
            status,
        )?;
        self.same_tree_records.insert(key, results.clone());
        Ok(results)
    }

    pub(crate) fn write_last_result_for_record(
        &mut self,
        root: &Path,
        checked_tree_oid: &str,
        expectation: &SelectedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        self.write_last_result_for_record_inner(root, checked_tree_oid, expectation, record)
    }

    pub(crate) fn write_last_result_for_record_or_absent_history(
        &mut self,
        root: Option<&Path>,
        checked_tree_oid: &str,
        expectation: &SelectedExpectation,
        record: &CheckRecord,
    ) -> Result<Option<LastResult>, String> {
        let Some(root) = root else {
            // Last Results are file-backed xpec state under XPECS_DIR. A
            // runtime with absent persistent history has no status-specific
            // files to update.
            return Ok(None);
        };
        self.write_last_result_for_record(root, checked_tree_oid, expectation, record)
            .map(Some)
    }

    fn write_last_result_for_record_inner(
        &mut self,
        root: &Path,
        checked_tree_oid: &str,
        expectation: &SelectedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        let status = last_result_status_for_record(expectation, record);
        let now = format_record_timestamp(unix_timestamp()?);
        let q_scope = record.scope.clone();
        let visible_scope = visible_scope(&expectation.agent, &q_scope)?;
        let result = LastResult {
            response_timestamp: record.timestamp.clone(),
            updated_timestamp: now,
            status,
            response: normalized_response_from_record(record),
            q_scope: q_scope.clone(),
            visible_scope,
            checked_tree_oid: (status == LastResultStatus::Pass)
                .then(|| checked_tree_oid.to_string()),
            visible_tree_oid: matches!(status, LastResultStatus::Pass | LastResultStatus::Fail)
                .then(|| {
                    visible_tree_oid_for_persisted_scope(
                        root,
                        checked_tree_oid,
                        expectation,
                        record,
                        &q_scope,
                    )
                })
                .transpose()?,
        };
        self.write_last_result(root, expectation, &result)?;
        Ok(result)
    }

    pub(crate) fn refresh_last_result_for_checked_tree(
        &mut self,
        root: &Path,
        current_checked_tree_oid: &str,
        expectation: &SelectedExpectation,
        result: &LastResult,
    ) -> Result<LastResult, String> {
        let mut refreshed = result.clone();
        refreshed.updated_timestamp = format_record_timestamp(unix_timestamp()?);
        refreshed.checked_tree_oid = (refreshed.status == LastResultStatus::Pass)
            .then(|| current_checked_tree_oid.to_string());
        self.write_last_result(root, expectation, &refreshed)?;
        Ok(refreshed)
    }

    fn write_last_result(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        result: &LastResult,
    ) -> Result<(), String> {
        validate_last_result(result.status, result)?;
        let path = self.last_result_path(root, expectation, result.status)?;
        let temp_path = temp_path_for(&path)?;
        self.save_replaced_same_tree_record(root, expectation, result.status, &path, result)?;
        // Last-result files are whole-record snapshots. Refreshing a result
        // writes one complete newly persisted status record plus the `last.json`
        // alias below; it never rewrites an accumulated log or state prefix.
        write_temp_file_then_replace(&temp_path, &path, |file| {
            serde_json::to_writer(&mut *file, result)
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            std::io::Write::write_all(file, b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
        })?;
        refresh_last_json_link(&path, &self.xpec_dir(root, expectation)?.join("last.json"))?;
        self.last_results.insert(
            (root.to_path_buf(), expectation.id.clone(), result.status),
            Some(result.clone()),
        );
        Ok(())
    }

    fn last_result_path(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
    ) -> Result<PathBuf, String> {
        Ok(self.xpec_dir(root, expectation)?.join(status.file_name()))
    }

    fn same_tree_records_dir(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
    ) -> Result<PathBuf, String> {
        Ok(self
            .xpec_dir(root, expectation)?
            .join("same-tree-records")
            .join(status.as_str()))
    }

    fn save_replaced_same_tree_record(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
        status_path: &Path,
        replacement: &LastResult,
    ) -> Result<(), String> {
        if !matches!(status, LastResultStatus::Pass | LastResultStatus::Fail) {
            return Ok(());
        }
        let Some(previous) = read_last_result_path(status_path, status)? else {
            return Ok(());
        };
        if !should_save_same_tree_record(&previous, replacement) {
            return Ok(());
        }
        let dir = self.same_tree_records_dir(root, expectation, status)?;
        ensure_dir_without_symlinks(&dir)?;
        let path = dir.join(same_tree_record_file_name(&previous)?);
        if !same_tree_record_is_newer_than_existing(&path, status, &previous)? {
            return Ok(());
        }
        let temp_path = temp_path_for(&path)?;
        write_temp_file_then_replace(&temp_path, &path, |file| {
            serde_json::to_writer(&mut *file, &previous)
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            std::io::Write::write_all(file, b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
        })?;
        let key = (root.to_path_buf(), expectation.id.clone(), status);
        self.same_tree_records.remove(&key);
        Ok(())
    }
}

pub(super) fn check_record_from_last_result(
    expectation: &SelectedExpectation,
    result: &LastResult,
) -> CheckRecord {
    let error = result.error().map(str::to_string);
    let response_question_scope_suggestion = result.question_scope_suggestion();
    let observed = result
        .answer()
        .or_else(|| result.error())
        .unwrap_or("")
        .to_string();
    CheckRecord {
        timestamp: result.response_timestamp.clone(),
        number: expectation.number,
        result: result.status.check_result(),
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed,
        error,
        evidence: result.evidence(),
        scope: result.q_scope.clone(),
        question_scope_suggestion: response_question_scope_suggestion,
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

pub(super) fn pass_record_from_cooldown_result(
    expectation: &SelectedExpectation,
    result: &LastResult,
) -> CheckRecord {
    let response_question_scope_suggestion = result.question_scope_suggestion();
    CheckRecord {
        timestamp: result.response_timestamp.clone(),
        number: expectation.number,
        result: CheckResult::Pass,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed: expectation.expected_answer.clone(),
        error: None,
        evidence: result.evidence(),
        scope: result.q_scope.clone(),
        question_scope_suggestion: response_question_scope_suggestion,
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

fn last_result_status_for_record(
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> LastResultStatus {
    // Last-result status follows the final response shape: error responses and
    // technical failures have `error`; every present answer is pass or fail.
    if record.error.is_some() {
        LastResultStatus::Error
    } else if record.observed == expectation.expected_answer {
        LastResultStatus::Pass
    } else {
        LastResultStatus::Fail
    }
}

fn normalized_response_from_record(record: &CheckRecord) -> Value {
    let mut response = serde_json::Map::new();
    if let Some(error) = record.error.as_deref() {
        response.insert("error".to_string(), json!(error));
    } else {
        response.insert("answer".to_string(), json!(record.observed));
    }
    response.insert("evidence".to_string(), json!(record.evidence));
    if let Some(suggestion) = record.question_scope_suggestion.as_deref() {
        assert!(
            !suggestion.is_empty(),
            "qScopeSuggestion must be non-empty when present"
        );
        response.insert("qScopeSuggestion".to_string(), json!(suggestion));
    }
    Value::Object(response)
}

fn visible_tree_oid_for_persisted_scope(
    root: &Path,
    checked_tree_oid: &str,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    q_scope: &[String],
) -> Result<String, String> {
    if q_scope == record.scope.as_slice() {
        return Ok(record.visible_tree_oid.clone());
    }
    let checked_source = TreeSource::Git {
        treeish: checked_tree_oid.to_string(),
        tree_oid: checked_tree_oid.to_string(),
    };
    VisibleTreeOidCache::new()
        .visible_tree_oid(root, &checked_source, &expectation.agent, q_scope)
        .map_err(|err| format!("failed to hash persisted q-scope: {}", err))
}

fn read_last_result_path(
    path: &Path,
    expected_status: LastResultStatus,
) -> Result<Option<LastResult>, String> {
    reject_symlink(path)?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to read {}: {}", path.display(), err)),
    };
    let result = serde_json::from_str::<LastResult>(&content)
        .map_err(|err| format!("invalid last-result JSON in {}: {}", path.display(), err))?;
    validate_last_result(expected_status, &result).map_err(|message| {
        format!(
            "invalid last-result JSON in {}: {}",
            path.display(),
            message
        )
    })?;
    Ok(Some(result))
}

fn validate_last_result(
    expected_status: LastResultStatus,
    result: &LastResult,
) -> Result<(), String> {
    if result.status != expected_status {
        return Err(format!(
            "status must be {} for {}",
            expected_status.as_str(),
            expected_status.file_name()
        ));
    }
    if parse_record_timestamp(&result.response_timestamp).is_none() {
        return Err("responseTimestamp must be UTC in YYYY-MM-DDTHH:MM:SSZ form".to_string());
    }
    if parse_record_timestamp(&result.updated_timestamp).is_none() {
        return Err("updatedTimestamp must be UTC in YYYY-MM-DDTHH:MM:SSZ form".to_string());
    }
    if result
        .response
        .get("evidence")
        .and_then(Value::as_str)
        .is_none()
    {
        return Err("response must contain evidence".to_string());
    }
    if let Some(suggestion) = result.response.get("qScopeSuggestion") {
        validate_response_question_scope_suggestion(suggestion)?;
    }
    match expected_status {
        LastResultStatus::Pass => {
            if result.answer().is_none() {
                return Err("pass response must contain answer".to_string());
            }
            if result.error().is_some() {
                return Err("pass response must omit error".to_string());
            }
            if result.checked_tree_oid.is_none() {
                return Err("pass must contain checkedTreeOid".to_string());
            }
            if result.visible_tree_oid.is_none() {
                return Err("pass must contain visibleTreeOid".to_string());
            }
        }
        LastResultStatus::Fail => {
            if result.answer().is_none() {
                return Err("fail response must contain answer".to_string());
            }
            if result.error().is_some() {
                return Err("fail response must omit error".to_string());
            }
            if result.checked_tree_oid.is_some() {
                return Err("fail must omit checkedTreeOid".to_string());
            }
            if result.visible_tree_oid.is_none() {
                return Err("fail must contain visibleTreeOid".to_string());
            }
        }
        LastResultStatus::Error => {
            if result.answer().is_some() {
                return Err("error response must not contain answer".to_string());
            }
            if result.error().is_none() {
                return Err("error response must contain error".to_string());
            }
            if result.checked_tree_oid.is_some() {
                return Err("error must omit checkedTreeOid".to_string());
            }
            if result.visible_tree_oid.is_some() {
                return Err("error must omit visibleTreeOid".to_string());
            }
        }
    }
    Ok(())
}

fn validate_response_question_scope_suggestion(value: &Value) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err("response qScopeSuggestion must be an array".to_string());
    };
    if items.is_empty() {
        return Err("response qScopeSuggestion must be non-empty".to_string());
    }
    for item in items {
        let Some(path) = item.as_str() else {
            return Err("response qScopeSuggestion items must be strings".to_string());
        };
        if path.is_empty() || path.contains(['\r', '\n']) {
            return Err(
                "response qScopeSuggestion items must be non-empty single-line strings".to_string(),
            );
        }
    }
    Ok(())
}

fn should_save_same_tree_record(previous: &LastResult, replacement: &LastResult) -> bool {
    previous.status != replacement.status
        || previous.response_timestamp != replacement.response_timestamp
        || previous.response != replacement.response
        || previous.q_scope != replacement.q_scope
        || previous.visible_scope != replacement.visible_scope
        || previous.visible_tree_oid != replacement.visible_tree_oid
}

fn same_tree_record_file_name(result: &LastResult) -> Result<String, String> {
    let Some(visible_tree_oid) = result.visible_tree_oid.as_deref() else {
        return Err("same-tree record must contain visibleTreeOid".to_string());
    };
    // Same-tree history keeps the latest record for each retained visible
    // tree/scope pair instead of appending one file per replacement.
    let key = serde_json::to_vec(&json!({
        "visibleScope": result.visible_scope,
        "visibleTreeOid": visible_tree_oid,
    }))
    .map_err(|err| format!("failed to serialize same-tree record key: {}", err))?;
    Ok(format!(
        "{}-{}.json",
        path_safe_timestamp(visible_tree_oid),
        hash_60(&key)
    ))
}

fn path_safe_timestamp(timestamp: &str) -> String {
    timestamp
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn read_same_tree_records_dir(
    dir: &Path,
    status: LastResultStatus,
) -> Result<Vec<LastResult>, String> {
    reject_symlink(dir)?;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("failed to read {}: {}", dir.display(), err)),
    };
    let mut results = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(result) = read_last_result_path(&path, status)? {
            results.push(result);
        }
    }
    Ok(results)
}

fn same_tree_record_is_newer_than_existing(
    path: &Path,
    status: LastResultStatus,
    candidate: &LastResult,
) -> Result<bool, String> {
    let Some(existing) = read_last_result_path(path, status)? else {
        return Ok(true);
    };
    let existing_time = parse_record_timestamp(&existing.response_timestamp).unwrap_or(0);
    let candidate_time = parse_record_timestamp(&candidate.response_timestamp).unwrap_or(0);
    Ok(candidate_time > existing_time)
}

fn refresh_last_json_link(status_path: &Path, last_path: &Path) -> Result<(), String> {
    if let Some(parent) = last_path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(last_path)?;
    let temp_path = temp_path_for(last_path)?;
    match fs::hard_link(status_path, &temp_path) {
        Ok(()) => crate::fs_util::replace_file_with_temp(&temp_path, last_path),
        Err(hard_link_error) => {
            fs::copy(status_path, &temp_path).map_err(|copy_error| {
                let _ = fs::remove_file(&temp_path);
                format!(
                    "failed to hardlink {} to {}: {}; failed to copy instead: {}",
                    status_path.display(),
                    last_path.display(),
                    hard_link_error,
                    copy_error
                )
            })?;
            crate::fs_util::replace_file_with_temp(&temp_path, last_path)
        }
    }
}

fn temp_path_for(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("state path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = LAST_RESULT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
