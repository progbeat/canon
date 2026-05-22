use crate::check_types::{CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::scope_hash::ScopeHashCache;
use crate::UNPARSEABLE_OBSERVED;
use std::path::Path;

pub(crate) fn error_record_from_interrogation_error(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    scope: &[String],
    error: &str,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<CheckRecord, String> {
    let scope_hash = scope_hash_cache.staged_scope_hash(root, agent, scope)?;
    CheckRecord::current_from_expectation(
        agent,
        expectation,
        CheckRecordOutcome {
            result: CheckResult::Fail,
            observed: UNPARSEABLE_OBSERVED.to_string(),
            evidence: error.to_string(),
            scope: scope.to_vec(),
            scope_hash,
        },
    )
}
