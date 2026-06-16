use super::order::order_by_latest_non_pass;
use crate::check::core::{CheckOptions, SelectedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) struct CacheFilteredCheckWork {
    // Expectations that still require evaluator work. Cached hits are excluded
    // from this queue and are not selected evaluations.
    pub(crate) to_evaluate: Vec<SelectedExpectation>,
    pub(crate) cached_hits: Vec<CachedExpectationHit>,
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
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// Cache filtering receives command-independent `CheckOptions`; CLI/trailer
// policy stays in the command layer before and after this selection step.
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
        // Cached results come only from pass/fail same-tree or cooldown state;
        // last-error human-review records remain non-pass history for ordering,
        // but are not cache hits.
        match cached_result_for_expectation(
            context.root,
            context.source,
            &expectation.agent,
            &expectation,
            &mut *context.xpec_state,
            &mut *context.visible_tree_oid_cache,
            CachedResultLookup {
                now,
                include_same_tree: true,
                include_cooldown: !options.ignore_cooldown,
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
        // Selected Expectations default policy: when a no-selector run sees a
        // cached failure, the selected queue is empty until that cached failure
        // is fixed. This is before evaluation starts, so canon-check-order's
        // evaluated-expectation ordering and stop-after-non-pass rule is not
        // reached for the cleared expectations.
        to_evaluate.clear();
        cached_hits =
            order_by_latest_non_pass(context.root, cached_hits, context.xpec_state, |hit| {
                &hit.expectation
            })?;
    }
    Ok(CacheFilteredCheckWork {
        to_evaluate,
        cached_hits,
    })
}
