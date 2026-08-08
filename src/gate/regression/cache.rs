use crate::check::ResolvedExpectation;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::xpec_state::{
    cached_pass_result_for_expectation, refresh_reused_same_tree_pass_result,
    response_timestamp_is_within_cooldown, stored_visible_scope_matches_checked_tree,
    CachedPassResultLookup, GateHistory, XpecStateCache,
};
use std::path::Path;

pub(super) fn count(
    root: &Path,
    selected_expectations: &[ResolvedExpectation],
    trees: ComparisonTrees<'_>,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<usize, String> {
    // This is the gate spec's only expectation-related failure: a pass on the
    // baseline tree followed by a fail on the staged tree. Other non-OK check
    // results remain non-blocking.
    selected_expectations
        .iter()
        .map(|expectation| {
            Ok(expectation_status(
                root,
                expectation,
                trees,
                xpec_state,
                visible_tree_oid_cache,
                now,
            )?
            .is_blocking() as usize)
        })
        .sum()
}

fn expectation_status(
    root: &Path,
    expectation: &ResolvedExpectation,
    trees: ComparisonTrees<'_>,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<ExpectationStatus, String> {
    let previous = tree_result_at(
        root,
        expectation,
        trees.previous,
        xpec_state,
        visible_tree_oid_cache,
        now,
    )?;
    let current = tree_result_at(
        root,
        expectation,
        trees.current,
        xpec_state,
        visible_tree_oid_cache,
        now,
    )?;
    Ok(match (previous, current) {
        (TreeResult::Pass, TreeResult::Fail) => ExpectationStatus::Regressed,
        _ => ExpectationStatus::PassedOrNonBlocking,
    })
}

enum ExpectationStatus {
    PassedOrNonBlocking,
    Regressed,
}

impl ExpectationStatus {
    fn is_blocking(&self) -> bool {
        matches!(self, ExpectationStatus::Regressed)
    }
}

#[derive(Debug, Clone)]
enum TreeResult {
    Pass,
    Fail,
    Missing,
}

fn tree_result_at(
    root: &Path,
    expectation: &ResolvedExpectation,
    tree: ComparisonTree<'_>,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<TreeResult, String> {
    // [KD,cw,90,Sh] Gate history is not canonical Last Results and is not a
    // Cached Result. It retains only the fields needed to compare Git-backed
    // gate outcomes after an in-place check has replaced the canonical files.
    // A matching history entry therefore must not refresh `last-pass.json`.
    if let Some(gate_history) = xpec_state.read_gate_results(root, expectation)? {
        return tree_result_from_gate_history(
            root,
            expectation,
            tree,
            visible_tree_oid_cache,
            now,
            gate_history,
        );
    }
    // [KD,cw,g2,Sh] A repository created before the dedicated Git-backed gate
    // history may still have canonical Git-backed Last Results. This fallback
    // is the Cached Result path: refreshing a reused same-tree pass updates the
    // required bounded cross-invocation Last Results and gate history; no
    // invocation-local memo table is persisted.
    let hit = cached_pass_result_for_expectation(
        root,
        tree.source,
        expectation,
        xpec_state,
        visible_tree_oid_cache,
        CachedPassResultLookup {
            now,
            include_same_tree: true,
            include_cooldown: true,
        },
    )?;
    if let Some(hit) = hit {
        refresh_reused_same_tree_pass_result(root, tree.tree_oid, expectation, xpec_state, hit)?;
        // xpec: m
        // `cached_pass_result_for_expectation` can return only a pass.
        return Ok(TreeResult::Pass);
    }
    Ok(result_from_failed_tree_oid(
        xpec_state
            .read_last_fail(root, expectation)?
            .and_then(|result| result.checked_tree_oid),
        tree.tree_oid,
    ))
}

fn tree_result_from_gate_history(
    root: &Path,
    expectation: &ResolvedExpectation,
    tree: ComparisonTree<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
    gate_history: GateHistory,
) -> Result<TreeResult, String> {
    if let Some(historical_git_backed_pass) = gate_history.last_pass {
        let historical_pass_matches_tree = stored_visible_scope_matches_checked_tree(
            root,
            tree.source,
            &historical_git_backed_pass.visible_scope,
            Some(&historical_git_backed_pass.visible_tree_oid),
            visible_tree_oid_cache,
        )?;
        let historical_pass_is_within_cooldown = response_timestamp_is_within_cooldown(
            expectation,
            &historical_git_backed_pass.response_timestamp,
            now,
        );
        if historical_pass_matches_tree || historical_pass_is_within_cooldown {
            // [W4,Sh] A matching Git-backed gate-history pass is authoritative
            // before the older failed-tree record below is considered. This is
            // regression history, not reuse of the canonical same-tree result;
            // it intentionally does not update canonical Last Results.
            return Ok(TreeResult::Pass);
        }
    }
    Ok(result_from_failed_tree_oid(
        gate_history.last_fail.map(|result| result.checked_tree_oid),
        tree.tree_oid,
    ))
}

fn result_from_failed_tree_oid(failed_tree_oid: Option<String>, tree_oid: &str) -> TreeResult {
    if failed_tree_oid.is_some_and(|failed_tree_oid| failed_tree_oid == tree_oid) {
        TreeResult::Fail
    } else {
        TreeResult::Missing
    }
}

#[derive(Clone, Copy)]
pub(super) struct ComparisonTree<'a> {
    pub(super) source: &'a TreeSource,
    pub(super) tree_oid: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct ComparisonTrees<'a> {
    pub(super) previous: ComparisonTree<'a>,
    pub(super) current: ComparisonTree<'a>,
}
