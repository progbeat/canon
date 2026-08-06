use super::order::order_selected_by_rank_and_latest_fail;
use crate::check::core::{CheckOptions, ResolvedExpectation};
use crate::check::engine::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) struct GitBackedCacheFilteredCheckWork {
    pub(crate) selected_evaluation_queue: Vec<ResolvedExpectation>,
    pub(crate) reused_non_selected_results: Vec<CheckCacheHit>,
}

pub(crate) struct GitBackedCacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) checked_tree_oid: &'a str,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// [m,90,HS] This is the complete cached-result selection boundary. Cached
// Result is defined for an expectation and Git state, so both the public
// function and its context require a `TreeSource`; there is deliberately no
// in-place cache lookup API. Explicit Git-backed selections are forced but
// still ordered. In default mode, cache hits become reused, non-selected
// results; only cache misses become Selected expectations and enter the ordered
// evaluator queue. The reused results are report bookkeeping, not evaluations,
// and therefore are not members of the check-order sequence.
//
// Selection reads existing xpec state and may emit bounded runtime-log events
// through the supplied writer, but it does not create a persistent state family
// of its own.
//
// This function receives command-independent `CheckOptions`; CLI/trailer policy
// stays in the command layer before and after this selection step.
pub(crate) fn select_and_order_git_backed_expectations(
    context: GitBackedCacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
) -> Result<GitBackedCacheFilteredCheckWork, String> {
    let (selected_evaluation_queue, reused_non_selected_results) = if options.selectors_provided {
        // [2gZ] A forced selection may still have a cached result; Cached Result
        // defines that result, while Selected Expectations requires evaluation
        // anyway. Do not reuse the cached result or move the explicitly selected
        // xpec out of the evaluator queue.
        (options.candidate_expectations.clone(), Vec::new())
    } else {
        let mut selected_evaluation_queue = Vec::new();
        let mut reused_non_selected_results = Vec::new();
        for expectation in options.candidate_expectations.clone() {
            match cached_result_for_expectation(
                context.root,
                context.source,
                context.checked_tree_oid,
                &expectation,
                &mut *context.xpec_state,
                &mut *context.visible_tree_oid_cache,
                CachedResultLookup {
                    now,
                    include_same_tree: true,
                    include_cooldown: true,
                },
            )? {
                Some(hit) => {
                    if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                        write_cache_hit(writer, &hit)?;
                    }
                    reused_non_selected_results.push(hit);
                }
                None => selected_evaluation_queue.push(expectation),
            }
        }
        (selected_evaluation_queue, reused_non_selected_results)
    };
    let selected_evaluation_queue = order_selected_by_rank_and_latest_fail(
        context.root,
        selected_evaluation_queue,
        &mut *context.xpec_state,
        |expectation| expectation,
    )?;
    Ok(GitBackedCacheFilteredCheckWork {
        selected_evaluation_queue,
        reused_non_selected_results,
    })
}

#[cfg(test)]
mod tests;
