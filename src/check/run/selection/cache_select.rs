use super::order::order_by_latest_non_pass;
use crate::check::core::{CheckOptions, SelectedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) struct CacheFilteredCheckWork {
    pub(crate) to_evaluate: Vec<SelectedExpectation>,
    pub(crate) cached_hits: Vec<CachedExpectationHit>,
    pub(crate) cached_failure_seen: bool,
}

pub(crate) struct CachedExpectationHit {
    pub(crate) expectation: SelectedExpectation,
    pub(crate) hit: CheckCacheHit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedFailureMode {
    Continue,
    StopDefaultSelection,
}

pub(crate) struct CacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) history_cache: &'a mut HistoryCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) active_lazy_full_scope_reset_ids: &'a BTreeSet<String>,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

pub(crate) fn select_expectations_after_cache(
    context: CacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
    cached_failure_mode: CachedFailureMode,
) -> Result<CacheFilteredCheckWork, String> {
    let mut to_evaluate = Vec::new();
    let mut cached_hits = Vec::new();
    let mut cached_failure_seen = false;
    for expectation in options.selected.clone() {
        let active_lazy_full_scope_reset = context
            .active_lazy_full_scope_reset_ids
            .contains(&expectation.id);
        match cached_result_for_expectation(
            context.root,
            context.source,
            &expectation.agent,
            &expectation,
            &mut *context.history_cache,
            &mut *context.visible_tree_oid_cache,
            CachedResultLookup {
                now,
                include_same_tree: !active_lazy_full_scope_reset,
                include_cooldown: !options.ignore_cooldown && !active_lazy_full_scope_reset,
            },
        )? {
            Some(hit) => {
                cached_failure_seen |= !hit.record.passed();
                if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)?;
                }
                cached_hits.push(CachedExpectationHit { expectation, hit });
            }
            None => to_evaluate.push(expectation),
        }
    }
    if cached_failure_seen && cached_failure_mode == CachedFailureMode::StopDefaultSelection {
        to_evaluate.clear();
        cached_hits =
            order_by_latest_non_pass(context.root, cached_hits, context.history_cache, |hit| {
                &hit.expectation
            })?;
    }
    Ok(CacheFilteredCheckWork {
        to_evaluate,
        cached_hits,
        cached_failure_seen,
    })
}
