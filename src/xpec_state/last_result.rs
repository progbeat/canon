use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink, write_temp_file_then_replace};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
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
    #[serde(rename = "diffFrom", default, skip_serializing_if = "Option::is_none")]
    pub(crate) diff_from: Option<String>,
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

    pub(crate) fn question_scope_suggestion(&self) -> Option<Vec<String>> {
        self.response
            .get("qScopeSuggestion")?
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
    }
}

impl XpecStateCache {
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

    pub(crate) fn write_last_result_for_record(
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
            diff_from: Some(expectation.diff_from.clone()),
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

    pub(crate) fn refresh_last_result(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        result: &LastResult,
    ) -> Result<LastResult, String> {
        let mut refreshed = result.clone();
        refreshed.updated_timestamp = format_record_timestamp(unix_timestamp()?);
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
        if let Some(stale_status) = stale_answer_status_for(result.status) {
            self.remove_last_result(root, expectation, stale_status)?;
        }
        Ok(())
    }

    fn remove_last_result(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
        status: LastResultStatus,
    ) -> Result<(), String> {
        let path = self.last_result_path(root, expectation, status)?;
        reject_symlink(&path)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("failed to remove {}: {}", path.display(), err)),
        }
        self.last_results
            .insert((root.to_path_buf(), expectation.id.clone(), status), None);
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
}

fn stale_answer_status_for(status: LastResultStatus) -> Option<LastResultStatus> {
    match status {
        LastResultStatus::Pass => Some(LastResultStatus::Fail),
        LastResultStatus::Fail => Some(LastResultStatus::Pass),
        LastResultStatus::Error => None,
    }
}

pub(super) fn check_record_from_last_result(
    expectation: &SelectedExpectation,
    result: &LastResult,
) -> CheckRecord {
    let error = result.error().map(str::to_string);
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
        question_scope_suggestion: result.question_scope_suggestion(),
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

pub(super) fn pass_record_from_cooldown_result(
    expectation: &SelectedExpectation,
    result: &LastResult,
) -> CheckRecord {
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
        question_scope_suggestion: result.question_scope_suggestion(),
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

fn last_result_status_for_record(
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> LastResultStatus {
    // Records with `error` are the human-review path. They are stored in the
    // error status file so ordering and summaries can include them as non-pass
    // results without a separate persisted status.
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
    let suggestion = record
        .question_scope_suggestion
        .as_ref()
        .unwrap_or(&record.scope);
    response.insert("qScopeSuggestion".to_string(), json!(suggestion));
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
    let mut result = serde_json::from_str::<LastResult>(&content)
        .map_err(|err| format!("invalid last-result JSON in {}: {}", path.display(), err))?;
    normalize_legacy_last_result_response(&mut result);
    validate_last_result(expected_status, &result).map_err(|message| {
        format!(
            "invalid last-result JSON in {}: {}",
            path.display(),
            message
        )
    })?;
    Ok(Some(result))
}

fn normalize_legacy_last_result_response(result: &mut LastResult) {
    if result.question_scope_suggestion().is_some() {
        return;
    }
    if let Some(response) = result.response.as_object_mut() {
        let fallback_scope = if result.status == LastResultStatus::Pass {
            full_scope()
        } else {
            result.q_scope.clone()
        };
        response.insert("qScopeSuggestion".to_string(), json!(fallback_scope));
    }
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
    if result.question_scope_suggestion().is_none() {
        return Err("response must contain qScopeSuggestion".to_string());
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
