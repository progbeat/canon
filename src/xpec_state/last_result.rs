use crate::check::{CheckRecord, CheckResult, ResolvedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink, write_temp_file_then_replace};
use crate::git::{TreeSource, VisibleTreeOidCache};
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
}

impl LastResultStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "pass",
            LastResultStatus::Fail => "fail",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "last-pass.json",
            LastResultStatus::Fail => "last-fail.json",
        }
    }

    fn check_result(self) -> CheckResult {
        match self {
            LastResultStatus::Pass => CheckResult::Pass,
            LastResultStatus::Fail => CheckResult::Fail,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LastResult {
    // This struct is the persisted last-result schema. Git-backed evaluator
    // interrogation responses store the prompt-rendered diff base as
    // `diffFrom` and `diffFromTreeOid`; records from paths without such an
    // interrogation leave those fields absent. The containing xpec directory
    // is keyed by the full expectation ID; the JSON body does not persist the
    // expectation ID or human display prefix.
    #[serde(rename = "responseTimestamp")]
    pub(crate) response_timestamp: String,
    #[serde(rename = "updatedTimestamp")]
    pub(crate) updated_timestamp: String,
    pub(crate) status: LastResultStatus,
    pub(crate) response: Value,
    #[serde(rename = "qScope", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) q_scope: Vec<String>,
    #[serde(
        rename = "visibleScope",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
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
    #[serde(rename = "diffFrom", default, skip_serializing_if = "Option::is_none")]
    pub(crate) diff_from: Option<String>,
    #[serde(
        rename = "diffFromTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) diff_from_tree_oid: Option<String>,
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
        expectation: &ResolvedExpectation,
    ) -> Result<Option<Vec<String>>, String> {
        Ok(self
            .read_last_pass(root, expectation)?
            .map(|result| result.q_scope)
            .filter(|scope| !scope.is_empty()))
    }

    pub(crate) fn read_last_pass(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Pass)
    }

    pub(crate) fn read_last_fail(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<LastResult>, String> {
        self.read_last_result(root, expectation, LastResultStatus::Fail)
    }

    pub(crate) fn read_last_result(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
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
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        self.write_last_result_for_record_inner(root, checked_tree_oid, expectation, record)
    }

    pub(crate) fn write_interrogation_last_result_for_record_or_absent_history(
        &mut self,
        root: Option<&Path>,
        checked_tree_oid: &str,
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<Option<LastResult>, String> {
        let Some(root) = root else {
            // Last Results are file-backed xpec state under XPECS_DIR. A
            // runtime with absent persistent history has no status-specific
            // files to update.
            return Ok(None);
        };
        // Normal Git-backed check interrogations render a prompt diff, so this
        // interrogation-only writer requires the resolved diff provenance.
        // Lower-level writers still accept absent provenance for refreshed or
        // synthetic records that did not come from such an interrogation.
        if checked_tree_oid != "in-place" {
            require_git_backed_diff_provenance(
                record.diff_from.as_deref(),
                record.diff_from_tree_oid.as_deref(),
            )?;
        }
        self.write_last_result_for_record(root, checked_tree_oid, expectation, record)
            .map(Some)
    }

    pub(crate) fn write_last_result_for_record_or_absent_history(
        &mut self,
        root: Option<&Path>,
        checked_tree_oid: &str,
        expectation: &ResolvedExpectation,
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
        expectation: &ResolvedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        let status = last_result_status_for_record(expectation, record);
        let now = format_record_timestamp(unix_timestamp()?);
        let q_scope = record.scope.clone();
        let git_backed = checked_tree_oid != "in-place";
        let visible_scope = git_backed
            .then(|| visible_scope(&expectation.agent, &q_scope))
            .transpose()?;
        let result = LastResult {
            response_timestamp: record.timestamp.clone(),
            updated_timestamp: now,
            status,
            response: normalized_response_from_record(record),
            q_scope: if git_backed {
                q_scope.clone()
            } else {
                Vec::new()
            },
            visible_scope: visible_scope.unwrap_or_default(),
            checked_tree_oid: git_backed.then(|| checked_tree_oid.to_string()),
            visible_tree_oid: (git_backed && status == LastResultStatus::Pass)
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
            diff_from: record.diff_from.clone(),
            diff_from_tree_oid: record.diff_from_tree_oid.clone(),
        };
        self.write_last_result(root, expectation, &result)?;
        Ok(result)
    }

    pub(crate) fn refresh_last_result_for_checked_tree(
        &mut self,
        root: &Path,
        current_checked_tree_oid: &str,
        expectation: &ResolvedExpectation,
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
        expectation: &ResolvedExpectation,
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
        Ok(())
    }

    fn last_result_path(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        status: LastResultStatus,
    ) -> Result<PathBuf, String> {
        Ok(self.xpec_dir(root, expectation)?.join(status.file_name()))
    }
}

fn require_git_backed_diff_provenance(
    diff_from: Option<&str>,
    diff_from_tree_oid: Option<&str>,
) -> Result<(), String> {
    validate_optional_diff_provenance_pair(diff_from, diff_from_tree_oid)?;
    match (diff_from, diff_from_tree_oid) {
        (Some(_), Some(_)) => Ok(()),
        (None, None) => Err(
            "Git-backed interrogation last-result records must include diffFrom and diffFromTreeOid"
                .to_string(),
        ),
        (Some(_), None) | (None, Some(_)) => unreachable!(
            "validate_optional_diff_provenance_pair rejects partial diff provenance"
        ),
    }
}

fn validate_optional_diff_provenance_pair(
    diff_from: Option<&str>,
    diff_from_tree_oid: Option<&str>,
) -> Result<(), String> {
    match (diff_from, diff_from_tree_oid) {
        (Some(""), _) => Err("diffFrom must not be empty".to_string()),
        (_, Some("")) => Err("diffFromTreeOid must not be empty".to_string()),
        (Some(_), None) => Err("diffFromTreeOid is required with diffFrom".to_string()),
        (None, Some(_)) => Err("diffFrom is required with diffFromTreeOid".to_string()),
        (Some(_), Some(_)) | (None, None) => Ok(()),
    }
}

pub(super) fn check_record_from_last_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    result: &LastResult,
) -> Result<CheckRecord, String> {
    let error = result.error().map(str::to_string);
    let response_question_scope_suggestion = result.question_scope_suggestion();
    let observed = result
        .answer()
        .or_else(|| result.error())
        .unwrap_or("")
        .to_string();
    Ok(CheckRecord {
        timestamp: result.response_timestamp.clone(),
        number: expectation.number,
        result: result.status.check_result(),
        to: expectation.to,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed,
        error,
        evidence: result.evidence(),
        scope: if result.q_scope.is_empty() {
            crate::hash::full_scope()
        } else {
            result.q_scope.clone()
        },
        question_scope_suggestion: response_question_scope_suggestion,
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        diff_from: result.diff_from.clone(),
        diff_from_tree_oid: result.diff_from_tree_oid.clone(),
        diff_from_tree_oid_abbrev: diff_from_tree_oid_abbrev(root, result),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    })
}

pub(super) fn pass_record_from_cooldown_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    result: &LastResult,
) -> Result<CheckRecord, String> {
    let response_question_scope_suggestion = result.question_scope_suggestion();
    Ok(CheckRecord {
        timestamp: result.response_timestamp.clone(),
        number: expectation.number,
        result: CheckResult::Pass,
        to: expectation.to,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed: expectation.expected_answer.clone(),
        error: None,
        evidence: result.evidence(),
        scope: if result.q_scope.is_empty() {
            crate::hash::full_scope()
        } else {
            result.q_scope.clone()
        },
        question_scope_suggestion: response_question_scope_suggestion,
        visible_tree_oid: result.visible_tree_oid.clone().unwrap_or_default(),
        diff_from: result.diff_from.clone(),
        diff_from_tree_oid: result.diff_from_tree_oid.clone(),
        diff_from_tree_oid_abbrev: diff_from_tree_oid_abbrev(root, result),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    })
}

fn diff_from_tree_oid_abbrev(root: &Path, result: &LastResult) -> Option<String> {
    result.diff_from_tree_oid.as_deref().map(|oid| {
        crate::git::abbreviate_git_oid(root, oid)
            .unwrap_or_else(|_| fallback_diff_from_tree_oid_abbrev(oid))
    })
}

fn fallback_diff_from_tree_oid_abbrev(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn last_result_status_for_record(
    expectation: &ResolvedExpectation,
    record: &CheckRecord,
) -> LastResultStatus {
    if record.error.is_none() && record.observed == expectation.expected_answer {
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
    if record.error.is_none() {
        response.insert("evidence".to_string(), json!(record.evidence));
    }
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
    expectation: &ResolvedExpectation,
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
    let git_backed = result.checked_tree_oid.is_some();
    if git_backed == result.q_scope.is_empty() || git_backed == result.visible_scope.is_empty() {
        return Err(
            "qScope and visibleScope are required exactly for Git-backed results".to_string(),
        );
    }
    if let Some(suggestion) = result.response.get("qScopeSuggestion") {
        validate_response_question_scope_suggestion(suggestion)?;
    }
    // Reads of existing state and generic last-result writes can observe
    // optional diff provenance independently of the stricter Git-backed
    // interrogation writer, so schema validation rejects malformed partial
    // pairs while still allowing the pair to be absent.
    validate_optional_diff_provenance_pair(
        result.diff_from.as_deref(),
        result.diff_from_tree_oid.as_deref(),
    )?;
    match expected_status {
        LastResultStatus::Pass => {
            if result.answer().is_none() {
                return Err("pass response must contain answer".to_string());
            }
            if result.error().is_some() {
                return Err("pass response must omit error".to_string());
            }
            if git_backed && result.visible_tree_oid.is_none() {
                return Err("pass must contain visibleTreeOid".to_string());
            }
            if !git_backed && result.visible_tree_oid.is_some() {
                return Err("in-place pass must omit visibleTreeOid".to_string());
            }
        }
        LastResultStatus::Fail => {
            if result.answer().is_some() == result.error().is_some() {
                return Err("fail response must contain exactly one of answer or error".to_string());
            }
            if result.visible_tree_oid.is_some() {
                return Err("fail must omit visibleTreeOid".to_string());
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
