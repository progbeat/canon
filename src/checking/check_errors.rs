use crate::check_types::{CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::visible_tree_oid::VisibleTreeOidCache;
use crate::UNPARSEABLE_OBSERVED;
use std::path::Path;

pub(crate) fn error_record_from_interrogation_error(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let visible_tree_oid = visible_tree_oid_cache.staged_visible_tree_oid(root, agent, scope)?;
    CheckRecord::current_from_expectation(
        agent,
        expectation,
        CheckRecordOutcome {
            result: CheckResult::Fail,
            observed: UNPARSEABLE_OBSERVED.to_string(),
            evidence: error.to_string(),
            scope: scope.to_vec(),
            visible_tree_oid,
        },
    )
}
