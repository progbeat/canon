use crate::check_types::{CheckRecord, ObservedAnswerState, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::history::{HistoryCache, ReusableHistoryLookupKey};
use crate::scope::sanitize_scope_for_hash;
use crate::scope_hash::ScopeHashCache;
use crate::time::parse_record_timestamp;
use std::path::Path;

#[cfg(test)]
pub(crate) fn reusable_history_record(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<CheckRecord>, String> {
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    reusable_history_record_with_cache(
        root,
        agent,
        expectation,
        &mut history_cache,
        &mut scope_hash_cache,
    )
}

pub(crate) fn reusable_history_record_with_cache(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckRecord>, String> {
    // This is the answer-cache lookup described by the Cache spec: scan answer
    // history newest-to-oldest and accept only the first record whose stored
    // scopeTreeOid still matches the current staged contents for that scope.
    let key = ReusableHistoryLookupKey::new(root, expectation);
    if let Some(record) = history_cache.reusable_records.get(&key).cloned() {
        return Ok(record);
    }
    let reusable_record =
        latest_history_record_matching_hash(root, expectation, history_cache, |scope| {
            scope_hash_cache
                .staged_scope_hash(root, agent, scope)
                .map(Some)
        })?;
    history_cache
        .reusable_records
        .insert(key, reusable_record.clone());
    Ok(reusable_record)
}

pub(crate) fn latest_history_record_matching_hash(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut current_hash_for_scope: impl FnMut(&[String]) -> Result<Option<String>, String>,
) -> Result<Option<CheckRecord>, String> {
    // The hash match is deliberately tested before answer-shape validation so
    // the cache lookup follows the Cache spec's "first matching scopeTreeOid"
    // rule. The final validation only protects readers from legacy history
    // records that predate the current "answers only" write contract.
    let matched_record =
        scan_latest_history_records(root, expectation, history_cache, |mut record| {
            let Ok(scope) = sanitize_scope_for_hash(&record.scope) else {
                return Ok(HistoryRecordScan::Continue);
            };
            let Some(current_hash) = current_hash_for_scope(&scope)? else {
                return Ok(HistoryRecordScan::Continue);
            };
            if current_hash == record.scope_hash {
                record.scope = scope;
                return Ok(HistoryRecordScan::Done(Some(record)));
            }
            Ok(HistoryRecordScan::Continue)
        })?;
    let Some(record) = matched_record else {
        return Ok(None);
    };
    if !is_reusable_history_record_for_expected(&record, &expectation.a) {
        return Ok(None);
    }
    Ok(Some(record_with_current_expectation(record, expectation)))
}

pub(crate) fn cooldown_history_record(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Option<CheckRecord>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    scan_latest_history_records(root, expectation, history_cache, |record| {
        let Some(timestamp) = parse_record_timestamp(&record.timestamp) else {
            return Ok(HistoryRecordScan::Continue);
        };
        // Cooldown keys off the latest valid history record, not the latest
        // reusable answer record. This is why a newer fail or human-review-style
        // record blocks cooldown reuse of an older pass: after a failed check,
        // the old pass is no longer the spec-defined fresh cooldown pass.
        if !record.passed() {
            return Ok(HistoryRecordScan::Done(None));
        }
        if now.saturating_sub(timestamp) >= cooldown.seconds {
            return Ok(HistoryRecordScan::Done(None));
        }
        // Cooldown is not an answer-cache lookup and callers must not return
        // this record as a cached observed result. It is only the Cooldown spec's
        // selected-set filter: a fresh latest pass removes the expectation from
        // this invocation before exact-cache lookup or evaluator interrogation.
        Ok(HistoryRecordScan::Done(Some(record)))
    })
}

pub(crate) fn latest_history_scope_with_cache(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    // This returns only an enforced-scope seed for a fresh interrogation. It is
    // not a cached check result and does not let callers skip evaluator work.
    // The Interrogation Policy's "latest accepted scope" means the latest
    // reusable answer-history scope, not the latest passing scope. Verified
    // correct and incorrect answers both have accepted scopes; non-answer review
    // states such as `idk`, `malformed`, and unparseable responses do not.
    // This is only the starting scope for a fresh interrogation, regardless of
    // whether that old answer still matches the current staged tree for cache
    // reuse.
    scan_latest_history_records(root, expectation, history_cache, |record| {
        let Some(scope) = sanitized_reusable_history_scope(&record, &expectation.a) else {
            return Ok(HistoryRecordScan::Continue);
        };
        Ok(HistoryRecordScan::Done(Some(scope)))
    })
}

enum HistoryRecordScan<T> {
    Continue,
    Done(Option<T>),
}

fn scan_latest_history_records<T>(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut scan: impl FnMut(CheckRecord) -> Result<HistoryRecordScan<T>, String>,
) -> Result<Option<T>, String> {
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        match scan(record)? {
            HistoryRecordScan::Continue => {}
            HistoryRecordScan::Done(value) => return Ok(value),
        }
    }
    Ok(None)
}

fn sanitized_reusable_history_scope(record: &CheckRecord, expected: &str) -> Option<Vec<String>> {
    if !is_reusable_history_record_for_expected(record, expected) {
        return None;
    }
    sanitize_scope_for_hash(&record.scope).ok()
}

fn record_with_current_expectation(
    mut record: CheckRecord,
    expectation: &SelectedExpectation,
) -> CheckRecord {
    // The reusable lookup cache stores the raw matching history record. Current
    // display metadata is applied after lookup so moving or editing an
    // expectation during the same operation cannot make the cached value stale.
    record.id = expectation.id.clone();
    record.display_id = expectation.display_id.clone();
    record.number = expectation.number;
    record.prompt = Some(expectation.q.clone());
    record.expected = Some(expectation.a.clone());
    record
}

pub(crate) fn is_reusable_history_record(record: &CheckRecord) -> bool {
    record
        .expected_text()
        .is_some_and(|expected| is_reusable_history_record_for_expected(record, expected))
}

fn is_reusable_history_record_for_expected(record: &CheckRecord, expected: &str) -> bool {
    ObservedAnswerState::from_expected_and_observed(expected, &record.observed)
        .is_reusable_history()
}
