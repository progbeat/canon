use crate::check::core::types::{
    CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation, ERROR_UNPARSABLE,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::config_types::AgentConfig;
use crate::git::VisibleTreeOidCache;

pub(crate) fn error_record_from_interrogation_error(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    scope: &[String],
    error: &str,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    let visible_tree_oid = runtime.visible_tree_oid(visible_tree_oid_cache, agent, scope)?;
    CheckRecord::current_from_expectation(
        agent,
        expectation,
        CheckRecordOutcome {
            result: CheckResult::Fail,
            observed: ERROR_UNPARSABLE.to_string(),
            error: Some(ERROR_UNPARSABLE.to_string()),
            evidence: error.to_string(),
            scope: scope.to_vec(),
            suggested_q_scope: None,
            visible_tree_oid,
        },
    )
}
