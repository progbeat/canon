use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink, write_temp_file_then_replace};
use crate::git::{resolve_git_path, TreeSource, VisibleTreeOidCache};
use crate::scope::{q_scope_from_visible_scope, visible_scope};
use crate::state_paths::CANON_XPECS_DIR_GIT_PATH;
use crate::time::{format_record_timestamp, parse_record_timestamp, unix_timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

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

#[derive(Default)]
pub(crate) struct XpecStateCache {
    xpecs_dirs: BTreeMap<PathBuf, PathBuf>,
    xpec_dirs: BTreeMap<(PathBuf, String), PathBuf>,
    last_results: BTreeMap<LastResultCacheKey, Option<LastResult>>,
}

type LastResultCacheKey = (PathBuf, String, LastResultStatus);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedResultStatus {
    Pass,
    Fail,
}

pub(crate) struct CachedLastResultHit {
    pub(crate) result: LastResult,
    pub(crate) status: CachedResultStatus,
    pub(crate) kind: CachedLastResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedLastResultKind {
    SameTree,
    Cooldown,
}

impl XpecStateCache {
    pub(crate) fn xpecs_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.xpecs_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(root, CANON_XPECS_DIR_GIT_PATH)?;
        self.xpecs_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn xpec_dir(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.xpec_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = self.xpecs_dir(root)?.join(&expectation.id);
        self.xpec_dirs.insert(key, path.clone());
        Ok(path)
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

    pub(crate) fn write_last_result_for_record(
        &mut self,
        root: &Path,
        checked_tree_oid: &str,
        expectation: &SelectedExpectation,
        record: &CheckRecord,
    ) -> Result<LastResult, String> {
        let status = last_result_status_for_record(expectation, record);
        let now = format_record_timestamp(unix_timestamp()?);
        let result = LastResult {
            response_timestamp: record.timestamp.clone(),
            updated_timestamp: now,
            status,
            response: normalized_response_from_record(record),
            q_scope: record.scope.clone(),
            visible_scope: visible_scope(&expectation.agent, &record.scope)?,
            checked_tree_oid: (status == LastResultStatus::Pass)
                .then(|| checked_tree_oid.to_string()),
            visible_tree_oid: matches!(status, LastResultStatus::Pass | LastResultStatus::Fail)
                .then(|| record.visible_tree_oid.clone()),
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
}

pub(crate) fn snapshot_pass_ids(
    root: &Path,
    expectations: &[SelectedExpectation],
    cache: &mut XpecStateCache,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for expectation in expectations {
        if cache.read_last_pass(root, expectation)?.is_some() {
            ids.insert(expectation.id.clone());
        }
    }
    Ok(ids)
}

pub(crate) fn cached_last_result_for_expectation(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
    include_same_tree: bool,
    include_cooldown: bool,
) -> Result<Option<CachedLastResultHit>, String> {
    if include_same_tree {
        if let Some(result) = same_tree_last_result(
            root,
            source,
            &expectation.agent,
            expectation,
            state_cache,
            visible_tree_oid_cache,
        )? {
            let status = cached_status_for_same_tree(&result);
            let refreshed = state_cache.refresh_last_result(root, expectation, &result)?;
            return Ok(Some(CachedLastResultHit {
                result: refreshed,
                status,
                kind: CachedLastResultKind::SameTree,
            }));
        }
    }
    if include_cooldown {
        if let Some(result) = cooldown_last_result(root, expectation, state_cache, now)? {
            return Ok(Some(CachedLastResultHit {
                result,
                status: CachedResultStatus::Pass,
                kind: CachedLastResultKind::Cooldown,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn check_record_from_cached_result(
    expectation: &SelectedExpectation,
    hit: &CachedLastResultHit,
) -> CheckRecord {
    match hit.status {
        CachedResultStatus::Pass if hit.kind == CachedLastResultKind::Cooldown => {
            pass_record_from_cooldown_result(expectation, &hit.result)
        }
        CachedResultStatus::Pass | CachedResultStatus::Fail => {
            check_record_from_last_result(expectation, &hit.result)
        }
    }
}

pub(crate) fn latest_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    cache: &mut XpecStateCache,
) -> Result<Option<u64>, String> {
    let fail = cache.read_last_fail(root, expectation)?;
    let error = cache.read_last_error(root, expectation)?;
    Ok([fail, error]
        .into_iter()
        .flatten()
        .filter_map(|result| parse_record_timestamp(&result.updated_timestamp))
        .max())
}

pub(crate) fn cleanup_stale_xpec_dirs(
    xpecs_dir: &Path,
    active_ids: &BTreeSet<String>,
) -> Result<CacheCleanupStats, String> {
    if !xpecs_dir.exists() {
        return Ok(CacheCleanupStats {
            removed: 0,
            kept: 0,
        });
    }
    let mut stats = CacheCleanupStats {
        removed: 0,
        kept: 0,
    };
    for entry in fs::read_dir(xpecs_dir)
        .map_err(|err| format!("failed to read {}: {}", xpecs_dir.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", xpecs_dir.display(), err))?;
        let file_name = entry.file_name();
        let Some(id) = file_name.to_str() else {
            remove_state_entry(&entry.path())?;
            stats.removed += 1;
            continue;
        };
        if active_ids.contains(id) {
            stats.kept += 1;
        } else {
            remove_state_entry(&entry.path())?;
            stats.removed += 1;
        }
    }
    Ok(stats)
}

pub(crate) fn active_expectation_ids_from_identities(
    identities: &[crate::check::ExpectationIdentity],
) -> BTreeSet<String> {
    identities
        .iter()
        .map(|identity| identity.id.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CacheCleanupStats {
    pub(crate) removed: usize,
    pub(crate) kept: usize,
}

fn remove_state_entry(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    }
}

fn same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<LastResult>, String> {
    let resolver = visible_tree_oid_cache.reuse_resolver(root, source, agent)?;
    for status in [LastResultStatus::Fail, LastResultStatus::Pass] {
        let Some(result) = state_cache.read_last_result(root, expectation, status)? else {
            continue;
        };
        let Some(stored_visible_tree_oid) = result.visible_tree_oid.as_deref() else {
            continue;
        };
        let Ok(q_scope) = q_scope_from_visible_scope(agent, &result.visible_scope) else {
            continue;
        };
        let Some(current_visible_tree_oid) = resolver.visible_tree_oid_for_scope(&q_scope)? else {
            continue;
        };
        if current_visible_tree_oid == stored_visible_tree_oid {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

fn cooldown_last_result(
    root: &Path,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
) -> Result<Option<LastResult>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let latest = [LastResultStatus::Pass, LastResultStatus::Fail]
        .into_iter()
        .map(|status| state_cache.read_last_result(root, expectation, status))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .max_by_key(|result| parse_record_timestamp(&result.updated_timestamp).unwrap_or(0));
    let Some(result) = latest else {
        return Ok(None);
    };
    let Some(response_timestamp) = parse_record_timestamp(&result.response_timestamp) else {
        return Ok(None);
    };
    let check_result = match result.status {
        LastResultStatus::Pass => CheckResult::Pass,
        LastResultStatus::Fail => CheckResult::Fail,
        LastResultStatus::Error => return Ok(None),
    };
    let Some(duration) = cooldown.duration_for(check_result) else {
        return Ok(None);
    };
    if now.saturating_sub(response_timestamp) >= duration {
        return Ok(None);
    }
    Ok(Some(result))
}

fn cached_status_for_same_tree(result: &LastResult) -> CachedResultStatus {
    match result.status {
        LastResultStatus::Pass => CachedResultStatus::Pass,
        LastResultStatus::Fail | LastResultStatus::Error => CachedResultStatus::Fail,
    }
}

fn check_record_from_last_result(
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

fn pass_record_from_cooldown_result(
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
    if let Some(suggestion) = record.question_scope_suggestion.as_ref() {
        response.insert("qScopeSuggestion".to_string(), json!(suggestion));
    }
    Value::Object(response)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use crate::hash::full_scope;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn last_result_files_use_status_dependent_fields_and_last_json_follows_error() {
        let root = git_project("last-result-status-fields");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let scope = full_scope();

        let pass = test_record(&expectation, &scope, "yes", None);
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &pass)
            .unwrap();
        let pass_json = read_json(&root, &expectation.id, "last-pass.json");
        assert_eq!(pass_json["status"], "pass");
        assert_eq!(pass_json["checkedTreeOid"], "checked-tree");
        assert_eq!(pass_json["visibleTreeOid"], "visible-tree");

        let fail = test_record(&expectation, &scope, "no", None);
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &fail)
            .unwrap();
        let fail_json = read_json(&root, &expectation.id, "last-fail.json");
        assert_eq!(fail_json["status"], "fail");
        assert!(fail_json.get("checkedTreeOid").is_none());
        assert_eq!(fail_json["visibleTreeOid"], "visible-tree");

        let error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
            .unwrap();
        let error_json = read_json(&root, &expectation.id, "last-error.json");
        assert_eq!(error_json["status"], "error");
        assert!(error_json.get("checkedTreeOid").is_none());
        assert!(error_json.get("visibleTreeOid").is_none());

        let last_json = read_json(&root, &expectation.id, "last.json");
        assert_eq!(last_json, error_json);

        let _ = fs::remove_dir_all(root);
    }

    fn test_expectation() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "abc123".to_string(),
            display_id: "a".to_string(),
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            instructions: String::new(),
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }

    fn test_record(
        expectation: &SelectedExpectation,
        scope: &[String],
        observed: &str,
        error: Option<&str>,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: format_record_timestamp(1),
            number: expectation.number,
            result: CheckResult::from_expected_answer(&expectation.expected_answer, observed),
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "evidence".to_string(),
            scope: scope.to_vec(),
            question_scope_suggestion: Some(scope.to_vec()),
            visible_tree_oid: "visible-tree".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
        }
    }

    fn read_json(root: &Path, id: &str, file_name: &str) -> Value {
        let path = root
            .join(".git")
            .join("canon")
            .join("xpecs")
            .join(id)
            .join(file_name);
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }
}
