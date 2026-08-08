use crate::check::core::{CheckRecord, CheckResult, ResolvedExpectation};
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

impl InterrogationDiffProvenance {
    pub(crate) fn into_optional_record_fields(
        provenance: Option<InterrogationDiffProvenance>,
    ) -> (Option<String>, Option<String>, Option<String>) {
        provenance
            .map(|provenance| {
                (
                    Some(provenance.diff_from),
                    Some(provenance.diff_from_tree_oid),
                    Some(provenance.diff_from_tree_oid_abbrev),
                )
            })
            .unwrap_or((None, None, None))
    }
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
    let expected_answer = expectation.expected_answer().to_string();
    let (diff_from, diff_from_tree_oid, diff_from_tree_oid_abbrev) =
        InterrogationDiffProvenance::into_optional_record_fields(diff_provenance);
    Ok(CheckRecord {
        timestamp,
        result: CheckResult::Fail,
        to: expectation.to,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expected_answer),
        observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
        error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
        evidence: Some(error.to_string()),
        scope: scope.to_vec(),
        q_scope_suggestion: None,
        visible_tree_oid,
        diff_from,
        diff_from_tree_oid,
        diff_from_tree_oid_abbrev,
        id: expectation.require_configured_id()?.to_string(),
        display_id: expectation.display_id.clone(),
    })
}
