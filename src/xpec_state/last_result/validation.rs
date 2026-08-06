use super::{LastResult, LastResultStatus};
use crate::time::parse_record_timestamp;

pub(super) fn require_git_backed_diff_provenance(
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

pub(super) fn validate_git_fields_for_tree_context(
    checked_tree_oid: Option<&str>,
    diff_from: Option<&str>,
    diff_from_tree_oid: Option<&str>,
    visible_tree_oid: Option<&str>,
) -> Result<(), String> {
    validate_optional_diff_provenance_pair(diff_from, diff_from_tree_oid)?;
    if diff_from.is_some() && checked_tree_oid.is_none() {
        return Err("diffFrom and diffFromTreeOid require checkedTreeOid".to_string());
    }
    if visible_tree_oid.is_some() && checked_tree_oid.is_none() {
        return Err("visibleTreeOid requires checkedTreeOid".to_string());
    }
    Ok(())
}

pub(in crate::xpec_state) fn validate_last_result(
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
    // [Sh] A Git-backed result may omit diff provenance when its response did
    // not come from a diff-rendered evaluator interrogation. The inverse is
    // not true: a diff provenance pair proves Git backing and therefore
    // requires the complete Git-tree context.
    validate_git_fields_for_tree_context(
        result.checked_tree_oid.as_deref(),
        result.diff_from.as_deref(),
        result.diff_from_tree_oid.as_deref(),
        result.visible_tree_oid.as_deref(),
    )?;
    let git_backed = result.checked_tree_oid.is_some();
    if git_backed == result.q_scope.is_empty() || git_backed == result.visible_scope.is_empty() {
        return Err(
            "qScope and visibleScope are required exactly for Git-backed results".to_string(),
        );
    }
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
