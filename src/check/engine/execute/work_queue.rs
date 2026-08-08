use super::{CheckRunCaches, ResolveSelectedDiffFromTreeOids};
use crate::check::core::{CheckOptions, ResolvedExpectation};
use crate::check::engine::selection::{
    order_selected_by_rank_and_latest_fail,
    order_selected_when_every_expectation_has_no_fail_result,
    select_and_order_git_backed_expectations, GitBackedCacheFilterContext,
    GitBackedCacheFilteredCheckWork,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::logs::DiagnosticLogWriter;
use crate::repo_inspection::RepoInspectionCache;
use crate::time::unix_timestamp;
use std::path::Path;

pub(super) fn select_check_work(
    runtime: &CheckRuntime<'_>,
    options: &CheckOptions,
    caches: &mut CheckRunCaches,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
) -> Result<GitBackedCacheFilteredCheckWork, String> {
    if runtime.is_in_place() {
        let candidates = options.candidate_expectations.clone();
        let selected_evaluation_queue = if runtime.persistent_check_state_root().is_some() {
            select_and_order_in_place_expectations(
                runtime.root,
                candidates,
                &mut caches.xpec_state,
            )?
        } else {
            select_and_order_in_place_expectations_without_state(candidates)
        };
        return Ok(GitBackedCacheFilteredCheckWork {
            selected_evaluation_queue,
            reused_non_selected_results: Vec::new(),
        });
    }
    let source = runtime
        .tree_source()
        .ok_or_else(|| "missing Git tree source".to_string())?;
    let checked_tree_oid = runtime
        .git_checked_tree_oid()
        .ok_or_else(|| "missing checked tree OID".to_string())?;
    // Git-backed selection owns both cache filtering and final evaluation
    // ordering. Explicit selectors skip cache lookup but are still sorted by
    // rank and latest fail before the queue leaves this boundary.
    select_and_order_git_backed_expectations(
        GitBackedCacheFilterContext {
            root: runtime.root,
            source,
            checked_tree_oid,
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
            diagnostic_log,
        },
        options,
        unix_timestamp()?,
    )
}

pub(super) fn prepare_selected_diff_trees(
    runtime: &mut CheckRuntime<'_>,
    selected: &[ResolvedExpectation],
    repo_inspection: &mut RepoInspectionCache,
    resolve: Option<&mut ResolveSelectedDiffFromTreeOids<'_>>,
) -> Result<(), String> {
    let Some(resolve) = resolve else {
        return Ok(());
    };
    // [9h,Tv] This is the tree-resolution part of run preparation. Cache
    // filtering establishes the final Selected set first; resolve only that
    // set's symbolic prompt trees, once per distinct value, before any
    // evaluator work starts or any symbolic value reaches evaluation.
    let resolved = resolve(selected, repo_inspection)?;
    runtime.set_explicit_diff_from_tree_oids(resolved)
}

fn select_and_order_in_place_expectations(
    // This function contains only platform-independent selection ordering.
    // Filesystem and process variants stay behind platform-named modules.
    root: &Path,
    candidates: Vec<ResolvedExpectation>,
    last_result_history: &mut crate::xpec_state::XpecStateCache,
) -> Result<Vec<ResolvedExpectation>, String> {
    // [HS,m,90] Cached Result is defined only for an expectation plus
    // Git state. In-place mode has no Git state, so its Cached set is
    // structurally empty and both default and explicit selection retain every
    // CLI candidate. Persisted last-fail history affects only their order.
    order_selected_by_rank_and_latest_fail(root, candidates, last_result_history, |expectation| {
        expectation
    })
}

fn select_and_order_in_place_expectations_without_state(
    candidates: Vec<ResolvedExpectation>,
) -> Vec<ResolvedExpectation> {
    // [HS,m,90,H9] In-place still has no Cached Result domain. Without a
    // persistent state root it also has no last-result namespace, so every
    // selected candidate has the Unix epoch as its absent fail timestamp.
    order_selected_when_every_expectation_has_no_fail_result(candidates, |expectation| expectation)
}
