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
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
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
        to_evaluate.clear();
        cached_hits =
            order_by_latest_non_pass(context.root, cached_hits, context.xpec_state, |hit| {
                &hit.expectation
            })?;
    }
    Ok(CacheFilteredCheckWork {
        to_evaluate,
        cached_hits,
        cached_failure_seen,
    })
}
