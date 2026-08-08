//! Per-xpec last-result conversion, reuse, and file access.
//!
//! Namespace lifetime belongs to the same xpec-state component: it prunes
//! state outside the retained current configuration before enabling result
//! writes. In-place retention may keep only an uncollected ID's bounded
//! Git-backed gate cache.

mod model;
mod persistence;
mod validation;

use crate::check::{CheckRecord, CheckResult, ResolvedExpectation};
use std::path::Path;

pub(crate) use model::{LastResult, LastResultResponse, LastResultStatus};
pub(in crate::xpec_state) use persistence::{inspect_last_result_path, LastResultPathState};
pub(in crate::xpec_state) use validation::validate_last_result;

pub(super) fn check_record_from_last_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    result: &LastResult,
    visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let error = result.error().map(str::to_string);
    let observed = result
        .answer()
        .or_else(|| result.error())
        .unwrap_or("")
        .to_string();
    check_record_from_reused_result(
        root,
        expectation,
        result,
        result.status.check_result(),
        observed,
        error,
        visible_tree_oid_cache,
    )
}

pub(super) fn pass_record_from_cooldown_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    result: &LastResult,
    visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    check_record_from_reused_result(
        root,
        expectation,
        result,
        CheckResult::Pass,
        expectation.expected_answer().to_string(),
        None,
        visible_tree_oid_cache,
    )
}

fn check_record_from_reused_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    result: &LastResult,
    check_result: CheckResult,
    observed: String,
    error: Option<String>,
    visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let expected_answer = expectation.expected_answer().to_string();
    Ok(CheckRecord {
        timestamp: result.response_timestamp.clone(),
        result: check_result,
        to: expectation.to,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expected_answer),
        observed,
        error,
        evidence: result.evidence(),
        scope: if result.q_scope.is_empty() {
            crate::hash::full_scope()
        } else {
            result.q_scope.clone()
        },
        q_scope_suggestion: result.q_scope_suggestion(),
        visible_tree_oid: result.visible_tree_oid.clone(),
        diff_from: result.diff_from.clone(),
        diff_from_tree_oid: result.diff_from_tree_oid.clone(),
        diff_from_tree_oid_abbrev: diff_from_tree_oid_abbrev(root, result, visible_tree_oid_cache),
        id: expectation.require_configured_id()?.to_string(),
        display_id: expectation.display_id.clone(),
    })
}

fn diff_from_tree_oid_abbrev(
    root: &Path,
    result: &LastResult,
    visible_tree_oid_cache: &mut crate::git::VisibleTreeOidCache,
) -> Option<String> {
    result.diff_from_tree_oid.as_deref().map(|oid| {
        visible_tree_oid_cache
            .git_oid_abbreviation(root, oid)
            .unwrap_or_else(|_| fallback_diff_from_tree_oid_abbrev(oid))
    })
}

fn fallback_diff_from_tree_oid_abbrev(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn last_result_status_for_record(record: &CheckRecord) -> LastResultStatus {
    // [Sh] Check execution has already finalized the status. Persistence must
    // not independently reclassify it from response fields.
    match record.result {
        CheckResult::Pass => LastResultStatus::Pass,
        CheckResult::Fail => LastResultStatus::Fail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::ExpectationIdentity;
    use crate::config_types::{AgentConfig, ExpectationTo, DEFAULT_DIFF_FROM};
    use crate::xpec_state::XpecStateCache;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: 90,Sh
    fn in_place_writer_omits_git_tree_fields() {
        let result = write_in_place_result(in_place_fail_record()).unwrap();

        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["status"], "fail");
        assert_eq!(
            [
                json.get("qScope"),
                json.get("visibleScope"),
                json.get("checkedTreeOid"),
                json.get("visibleTreeOid"),
                json.get("diffFrom"),
                json.get("diffFromTreeOid"),
            ],
            [None, None, None, None, None, None]
        );
    }

    #[test] // xpec: T5,Sh,90
    fn in_place_writer_observes_forbidden_git_fields_before_normalization() {
        let mut diff_record = in_place_fail_record();
        diff_record.diff_from = Some(":checkpoint".to_string());
        diff_record.diff_from_tree_oid = Some("0123456789abcdef".to_string());
        assert!(write_in_place_result(diff_record).is_err());

        let mut visible_tree_record = in_place_fail_record();
        visible_tree_record.visible_tree_oid = Some("0123456789abcdef".to_string());
        assert!(write_in_place_result(visible_tree_record).is_err());
    }

    #[test] // xpec: Sh
    fn in_place_writer_does_not_use_q_scope_suggestion_as_a_decision_input() {
        let mut record = in_place_fail_record();
        record.q_scope_suggestion = Some(Vec::new());

        assert!(write_in_place_result(record).is_ok());
    }

    #[test] // xpec: Sh
    fn writer_uses_final_record_status_instead_of_reclassifying_the_answer() {
        let mut record = in_place_fail_record();
        record.observed = "yes".to_string();

        let result = write_in_place_result(record).unwrap();

        assert_eq!(result.status, LastResultStatus::Fail);
        assert_eq!(result.answer(), Some("yes"));
    }

    fn write_in_place_result(record: CheckRecord) -> Result<LastResult, String> {
        let root = temporary_git_root()?;
        let expectation = expectation();
        let identity = ExpectationIdentity {
            id: expectation.require_configured_id()?.to_string(),
            display_id: expectation.display_id.clone(),
        };
        let mut cache = XpecStateCache::default();
        cache.retain_only_current_configuration(&root, &[identity])?;
        let result = cache.write_last_result_for_record(&root, None, &expectation, &record);
        let _ = fs::remove_dir_all(root);
        result
    }

    fn in_place_fail_record() -> CheckRecord {
        CheckRecord {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            result: CheckResult::Fail,
            to: ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: "no".to_string(),
            error: None,
            evidence: Some("test evidence".to_string()),
            scope: vec![".".to_string()],
            q_scope_suggestion: None,
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "id".to_string(),
            display_id: "id".to_string(),
        }
    }

    fn expectation() -> ResolvedExpectation {
        ResolvedExpectation {
            kind: crate::check::ResolvedExpectationKind::Configured {
                id: "id".to_string(),
            },
            display_id: "id".to_string(),
            to: ExpectationTo::Agent,
            rank: 0,
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::default(),
            cooldown: None,
            q_scope: Default::default(),
        }
    }

    fn temporary_git_root() -> Result<PathBuf, String> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-last-result-component-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(root)
    }
}
