// Answer-history lookup for the Cache spec: newest-to-oldest history scanning
// plus current visibleTreeOid matching.
use crate::check_types::{CheckRecord, CheckResult, ObservedAnswerState, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::history::HistoryCache;
use crate::scope::sanitize_scope_for_hash;
use crate::time::parse_record_timestamp;
use crate::tree_source::TreeSource;
use crate::visible_tree_oid::VisibleTreeOidCache;
use std::path::Path;

#[cfg(test)]
pub(crate) fn same_tree_history_record(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<CheckRecord>, String> {
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut history_cache = HistoryCache::new();
    same_tree_history_record_with_cache(
        root,
        &TreeSource::Staged,
        agent,
        expectation,
        &mut history_cache,
        &mut visible_tree_oid_cache,
    )
}

pub(crate) fn same_tree_history_record_with_cache(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<CheckRecord>, String> {
    latest_history_record_matching_visible_tree_oid(root, expectation, history_cache, |scope| {
        visible_tree_oid_cache
            .visible_tree_oid(root, source, agent, scope)
            .map(Some)
    })
}

pub(crate) fn latest_history_record_matching_visible_tree_oid(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut current_visible_tree_oid_for_scope: impl FnMut(&[String]) -> Result<Option<String>, String>,
) -> Result<Option<CheckRecord>, String> {
    // Cache lookup follows the Cache spec's answer-history contract: only
    // schema-valid answer records loaded by the history reader can reach this
    // newest-to-oldest visibleTreeOid match.
    let matched_record =
        scan_latest_history_records(root, expectation, history_cache, |mut record| {
            if !is_reusable_history_record_for_expected(&record, &expectation.a) {
                return Ok(HistoryRecordScan::Continue);
            }
            let Ok(scope) = sanitize_scope_for_hash(&record.scope) else {
                return Ok(HistoryRecordScan::Continue);
            };
            let Some(current_visible_tree_oid) = current_visible_tree_oid_for_scope(&scope)? else {
                return Ok(HistoryRecordScan::Continue);
            };
            if current_visible_tree_oid == record.visible_tree_oid {
                record.scope = scope;
                return Ok(HistoryRecordScan::Done(Some(record)));
            }
            Ok(HistoryRecordScan::Continue)
        })?;
    Ok(matched_record.map(|record| record_with_current_expectation(record, expectation)))
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
    let record = scan_latest_history_records(root, expectation, history_cache, |mut record| {
        // Cooldown keys off the latest usable answer history record, unlike
        // same-tree lookup which searches for the latest visibleTreeOid match.
        // Legacy non-answer rows and invalid scopes are skipped here, while a
        // newer valid fail, bad timestamp, or expired pass deliberately blocks
        // cooldown reuse of an older pass.
        if !is_reusable_history_record_for_expected(&record, &expectation.a) {
            return Ok(HistoryRecordScan::Continue);
        }
        let Ok(scope) = sanitize_scope_for_hash(&record.scope) else {
            return Ok(HistoryRecordScan::Continue);
        };
        record.scope = scope;
        let Some(timestamp) = parse_record_timestamp(&record.timestamp) else {
            return Ok(HistoryRecordScan::Done(None));
        };
        if current_result_for_history_record(&record, expectation) != CheckResult::Pass {
            return Ok(HistoryRecordScan::Done(None));
        }
        if now.saturating_sub(timestamp) >= cooldown.seconds {
            return Ok(HistoryRecordScan::Done(None));
        }
        // Cooldown is not a same-tree lookup: a fresh latest pass can be the
        // cached result even when its visibleTreeOid differs from the current
        // evaluator-visible tree.
        Ok(HistoryRecordScan::Done(Some(record)))
    })?;
    Ok(record.map(|record| record_with_current_expectation(record, expectation)))
}

pub(crate) enum CachedHistoryRecord {
    SameTree(CheckRecord),
    Cooldown(CheckRecord),
}

pub(crate) fn newer_cached_history_record(
    same_tree: Option<CheckRecord>,
    cooldown: Option<CheckRecord>,
) -> Option<CachedHistoryRecord> {
    // Cached Result combines the same-tree and cooldown candidates by record
    // timestamp. The newer candidate wins; equal or unparsable timestamps keep
    // the same-tree result deterministic.
    match (same_tree, cooldown) {
        (Some(same_tree), Some(cooldown)) => {
            if record_timestamp_sort_key(&cooldown) > record_timestamp_sort_key(&same_tree) {
                Some(CachedHistoryRecord::Cooldown(cooldown))
            } else {
                Some(CachedHistoryRecord::SameTree(same_tree))
            }
        }
        (Some(record), None) => Some(CachedHistoryRecord::SameTree(record)),
        (None, Some(record)) => Some(CachedHistoryRecord::Cooldown(record)),
        (None, None) => None,
    }
}

pub(crate) fn latest_stored_q_scope_with_cache(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    // Expectation-mode `canon check` calls this before each fresh interrogation.
    // It returns only the latest stored q-scope from answer history; it is not a
    // cached check result and does not let callers skip evaluator work. Cache
    // specifies that answer-history records contain schema-valid `answer`
    // responses only, and each record's `qScope` is the q-scope used to form
    // that record's visible tree. Error and unparsable records are not answer
    // history records and cannot seed a fresh interrogation.
    scan_latest_history_records(root, expectation, history_cache, |record| {
        let Some(scope) = sanitized_answer_history_q_scope(&record, &expectation.a) else {
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

fn sanitized_answer_history_q_scope(record: &CheckRecord, expected: &str) -> Option<Vec<String>> {
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
    record.result = current_result_for_history_record(&record, expectation);
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

fn current_result_for_history_record(
    record: &CheckRecord,
    expectation: &SelectedExpectation,
) -> CheckResult {
    CheckResult::from_expected_answer(&expectation.a, &record.observed)
}

fn record_timestamp_sort_key(record: &CheckRecord) -> u64 {
    parse_record_timestamp(&record.timestamp).unwrap_or(0)
}
