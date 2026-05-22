use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::history::HistoryCache;
use crate::history_reuse::reusable_history_record_with_cache;
use crate::logging::DiagnosticLogWriter;
use crate::scope_hash::ScopeHashCache;
use serde_json::json;
use std::path::Path;

pub(crate) struct CheckCacheHit {
    pub(crate) record: CheckRecord,
}

pub(crate) struct FinalCacheSelection {
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) skipped_passes: Vec<(SelectedExpectation, CheckCacheHit)>,
}

pub(crate) fn cached_record_for_expectation(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckCacheHit>, String> {
    // The Cache spec's reusable-result lookup lives here. Cooldown filtering
    // has already happened before this call path; every hit returned from here
    // matched the current staged scopeTreeOid.
    reusable_history_record_with_cache(root, agent, expectation, history_cache, scope_hash_cache)
        .map(|record| record.map(|record| CheckCacheHit { record }))
}

pub(crate) fn cached_failure_for_expectation(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckCacheHit>, String> {
    Ok(
        cached_record_for_expectation(root, agent, expectation, history_cache, scope_hash_cache)?
            .filter(|hit| !hit.record.passed()),
    )
}

pub(crate) fn final_selected_after_current_pass_cache(
    root: &Path,
    agent: &AgentConfig,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<FinalCacheSelection, String> {
    let mut remaining = Vec::new();
    let mut skipped_passes = Vec::new();
    for expectation in selected {
        match cached_record_for_expectation(
            root,
            agent,
            &expectation,
            history_cache,
            scope_hash_cache,
        )? {
            Some(hit) if hit.record.passed() => skipped_passes.push((expectation, hit)),
            _ => remaining.push(expectation),
        }
    }
    Ok(FinalCacheSelection {
        selected: remaining,
        skipped_passes,
    })
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    writer
        .write_event(
            "info",
            "cache.exact_hit",
            &[
                ("id", json!(hit.record.id)),
                ("result", json!(hit.record.result)),
                ("scope", json!(hit.record.scope)),
            ],
        )
        .map_err(|err| err.to_string())?;
    writer
        .write_record(&hit.record)
        .map_err(|err| err.to_string())
}
