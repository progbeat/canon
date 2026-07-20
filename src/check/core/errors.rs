use crate::check::core::{CheckRecord, CheckResult, ResolvedExpectation};
use crate::check::interrogation::state::CheckRuntime;
use crate::config_types::AgentConfig;
use crate::git::VisibleTreeOidCache;
use crate::time::{format_record_timestamp, unix_timestamp};

// Internal normalized failure marker for technical evaluator failures that do
// not produce a schema-valid evaluator response.
pub(crate) const INTERNAL_ERROR_UNPARSABLE: &str = "unparsable";

#[derive(Debug, Clone)]
pub(crate) struct InterrogationDiffProvenance {
    pub(crate) diff_from: String,
    pub(crate) diff_from_tree_oid: String,
    pub(crate) diff_from_tree_oid_abbrev: String,
}

pub(crate) fn error_record_from_interrogation_error(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    diff_provenance: Option<InterrogationDiffProvenance>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let visible_tree_oid = runtime.visible_tree_oid(visible_tree_oid_cache, agent, scope)?;
    error_record_from_visible_tree_oid_with_diff_provenance(
        expectation,
        scope,
        error,
        visible_tree_oid,
        diff_provenance,
    )
}

pub(crate) fn error_record_from_visible_tree_oid(
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid: Option<String>,
) -> Result<CheckRecord, String> {
    error_record_from_visible_tree_oid_with_diff_provenance(
        expectation,
        scope,
        error,
        visible_tree_oid,
        None,
    )
}

pub(crate) fn error_record_from_visible_tree_oid_with_diff_provenance(
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid: Option<String>,
    diff_provenance: Option<InterrogationDiffProvenance>,
) -> Result<CheckRecord, String> {
    let timestamp = format_record_timestamp(unix_timestamp()?);
    Ok(error_record_from_visible_tree_oid_at(
        expectation,
        scope,
        error,
        visible_tree_oid,
        diff_provenance,
        timestamp,
    ))
}

pub(crate) fn error_record_from_visible_tree_oid_at(
    expectation: &ResolvedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid: Option<String>,
    diff_provenance: Option<InterrogationDiffProvenance>,
    timestamp: String,
) -> CheckRecord {
    let (diff_from, diff_from_tree_oid, diff_from_tree_oid_abbrev) = diff_provenance
        .map(|provenance| {
            (
                Some(provenance.diff_from),
                Some(provenance.diff_from_tree_oid),
                Some(provenance.diff_from_tree_oid_abbrev),
            )
        })
        .unwrap_or((None, None, None));
    CheckRecord {
        timestamp,
        number: expectation.number,
        result: CheckResult::Fail,
        to: expectation.to,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
        error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
        evidence: Some(error.to_string()),
        scope: scope.to_vec(),
        question_scope_suggestion: None,
        visible_tree_oid,
        diff_from,
        diff_from_tree_oid,
        diff_from_tree_oid_abbrev,
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}
