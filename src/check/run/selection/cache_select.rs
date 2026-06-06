use super::order::latest_non_pass_timestamp_with_cache;
use crate::check::core::types::{CheckOptions, SelectedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) struct CachedSelection {
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) cached: Vec<CachedSelectionHit>,
    pub(crate) cached_failure_seen: bool,
}

pub(crate) struct CachedSelectionHit {
    pub(crate) expectation: SelectedExpectation,
    pub(crate) hit: CheckCacheHit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedFailureMode {
    Continue,
    StopDefaultSelection,
}

pub(crate) struct CachedSelectionContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) history_cache: &'a mut HistoryCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) active_lazy_full_scope_reset_ids: &'a BTreeSet<String>,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

pub(crate) fn select_expectations_after_cache(
    context: CachedSelectionContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
    cached_failure_mode: CachedFailureMode,
) -> Result<CachedSelection, String> {
    let mut selected = Vec::new();
    let mut cached = Vec::new();
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
                cached.push(CachedSelectionHit { expectation, hit });
            }
            None => selected.push(expectation),
        }
    }
    if cached_failure_seen && cached_failure_mode == CachedFailureMode::StopDefaultSelection {
        selected.clear();
        cached = order_cached_failures_first(context.root, cached, context.history_cache)?;
    }
    Ok(CachedSelection {
        selected,
        cached,
        cached_failure_seen,
    })
}

fn order_cached_failures_first(
    root: &Path,
    cached: Vec<CachedSelectionHit>,
    history_cache: &mut HistoryCache,
) -> Result<Vec<CachedSelectionHit>, String> {
    let mut ordered_cached = cached
        .into_iter()
        .enumerate()
        .map(|(index, hit)| {
            Ok(OrderedCachedSelectionHit {
                latest_non_pass: latest_non_pass_timestamp_with_cache(
                    root,
                    &hit.expectation,
                    history_cache,
                )?,
                index,
                hit,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered_cached.sort_by(|left, right| {
        right
            .latest_non_pass
            .cmp(&left.latest_non_pass)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(ordered_cached
        .into_iter()
        .map(|ordered| ordered.hit)
        .collect())
}

struct OrderedCachedSelectionHit {
    hit: CachedSelectionHit,
    latest_non_pass: u64,
    index: usize,
}
