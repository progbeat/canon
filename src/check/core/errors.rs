use crate::check::core::{CheckRecord, CheckResult, ResolvedExpectation};
use crate::check::interrogation::state::CheckRuntime;
use crate::config_types::AgentConfig;
use crate::git::VisibleTreeOidCache;
use crate::time::{format_record_timestamp, unix_timestamp};

// Internal normalized failure marker for technical evaluator failures that do
// not produce a schema-valid evaluator response.
pub(crate) const INTERNAL_ERROR_UNPARSABLE: &str = "unparsable";

pub(crate) fn error_record_from_interrogation_error(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let visible_tree_oid = runtime.visible_tree_oid(visible_tree_oid_cache, agent, scope)?;
    error_record_from_visible_tree_oid(expectation, scope, error, visible_tree_oid)
}

pub(crate) fn error_record_from_visible_tree_oid(
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid: String,
) -> Result<CheckRecord, String> {
    let timestamp = format_record_timestamp(unix_timestamp()?);
    Ok(error_record_from_visible_tree_oid_at(
        expectation,
        scope,
        error,
        visible_tree_oid,
        timestamp,
    ))
}

pub(crate) fn error_record_from_visible_tree_oid_at(
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid: String,
    timestamp: String,
) -> CheckRecord {
    CheckRecord {
        timestamp,
        number: expectation.number,
        result: CheckResult::Fail,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
        error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
        evidence: error.to_string(),
        scope: scope.to_vec(),
        question_scope_suggestion: None,
        visible_tree_oid,
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}
