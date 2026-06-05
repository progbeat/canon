#[cfg(test)]
use crate::check::ExpectationIdentity;
use crate::check::{
    expectation_identities, is_canon_only_staged_change_bytes, is_canon_project_path_bytes,
    select_expectations_with_identities, staged_changed_path_bytes, SelectedExpectation,
    CHECK_PATH,
};
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::{
    cached_history_record, cooldown_history_record,
    latest_history_record_matching_visible_tree_oid, CachedHistoryRecord, HistoryCache,
};
use crate::output::write_stderr_line;
use crate::repo_inspection::RepoInspectionCache;
use crate::time::unix_timestamp;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    // CLI validation happens before the gate pass/fail decision. These
    // unsupported-option errors are usage errors, not `GateFailed` outcomes.
    if !args.is_empty() {
        return Err(
            "canon gate does not accept arguments\n▷ Run `canon gate` without arguments.".into(),
        );
    }
    // `canon check` prints "Commit the staged changes NOW!" only when this
    // same HEAD-vs-staged regression count is zero. In that same-tree commit
    // case, remaining expectation failures are not gate failures; only a
    // staged regression from HEAD pass to staged fail blocks the hook.
    let num_regressions = gate_result_or_failure(gate_regression_count(root))?;
    if num_regressions > 0 {
        write_gate_failure_event(GateFailureEvent::Regressed)?;
        return Err(CommandError::GateFailed);
    }
    if gate_result_or_failure(has_mixed_canon_and_non_canon_changes(root))? {
        write_stderr_line(
            "canon gate: .canon/** changes must not be mixed with non-.canon changes",
        )?;
        write_stderr_line("▷ Ask human to handle .canon/ changes.")?;
        return Err(CommandError::GateFailed);
    }
    Ok(())
}

fn gate_result_or_failure<T>(result: Result<T, String>) -> Result<T, CommandError> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            write_stderr_line(&format!("canon gate: {}", err))?;
            write_stderr_line(gate_error_advice())?;
            Err(CommandError::GateFailed)
        }
    }
}

fn gate_regression_count(root: &Path) -> Result<usize, String> {
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH), &TreeSource::Staged)?;
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut history_cache = HistoryCache::default();
    gate_regression_count_with_config(
        root,
        &config,
        &mut history_cache,
        &mut visible_tree_oid_cache,
    )
}

pub(crate) enum GateFailureEvent {
    Regressed,
}

fn has_mixed_canon_and_non_canon_changes(root: &Path) -> Result<bool, String> {
    let changed_paths = staged_changed_path_bytes(root)?;
    let has_canon_change = changed_paths
        .iter()
        .any(|path| is_canon_project_path_bytes(path));
    Ok(has_canon_change && !is_canon_only_staged_change_bytes(&changed_paths))
}

#[cfg(test)]
pub(crate) fn gate_pass_with_config(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    mut caches: GateCaches<'_>,
    now: u64,
    emit_failure: impl FnMut(GateFailureEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let selected_expectations = select_expectations_with_identities(config, identities, &[])?;
    // The gate pseudocode fails only on staged regressions. Missing staged
    // cached results are non-blocking; `canon check` is still responsible for
    // producing fresh history when the human wants full confirmation.
    gate_selected_expectations(
        root,
        &config.agent,
        &selected_expectations,
        &mut caches,
        now,
        emit_failure,
    )
}

pub(crate) fn gate_regression_count_with_config(
    root: &Path,
    config: &CheckConfig,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    // This is the gate spec's expectation comparison as a count: HEAD pass
    // followed by staged fail.
    let identities = expectation_identities(config)?;
    let selected_expectations = select_expectations_with_identities(config, &identities, &[])?;
    let now = unix_timestamp()?;
    selected_expectations
        .iter()
        .map(|expectation| {
            Ok(gate_expectation_status(
                root,
                &config.agent,
                expectation,
                history_cache,
                visible_tree_oid_cache,
                now,
            )?
            .is_blocking() as usize)
        })
        .sum()
}

#[cfg(test)]
fn gate_selected_expectations(
    root: &Path,
    agent: &AgentConfig,
    selected_expectations: &[SelectedExpectation],
    caches: &mut GateCaches<'_>,
    now: u64,
    mut emit_failure: impl FnMut(GateFailureEvent) -> Result<(), String>,
) -> Result<bool, String> {
    // These statuses are the only expectation-related way `canon gate` can
    // fail. Non-OK check results can remain non-blocking here when they did not
    // regress from a HEAD pass.
    // `gate_regression_count_with_config` counts these same statuses for
    // `canon check`'s `num_regressions`; if that count is zero, this loop has
    // no expectation-related failure branch to take.
    for expectation in selected_expectations {
        match gate_expectation_status(
            root,
            agent,
            expectation,
            caches.history,
            caches.visible_tree_oid,
            now,
        )? {
            GateExpectationStatus::PassedOrNonBlocking => {}
            GateExpectationStatus::Regressed => {
                emit_failure(GateFailureEvent::Regressed)?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn gate_expectation_status(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<GateExpectationStatus, String> {
    let previous = gate_cache_result_for_tree_at(
        root,
        agent,
        expectation,
        GateComparisonTree::Head,
        history_cache,
        visible_tree_oid_cache,
        now,
    )?;
    let current = gate_cache_result_for_tree_at(
        root,
        agent,
        expectation,
        GateComparisonTree::StagedIndex,
        history_cache,
        visible_tree_oid_cache,
        now,
    )?;
    Ok(match (previous, current) {
        (GateCacheResult::Pass, GateCacheResult::Fail) => GateExpectationStatus::Regressed,
        _ => GateExpectationStatus::PassedOrNonBlocking,
    })
}

enum GateExpectationStatus {
    PassedOrNonBlocking,
    Regressed,
}

impl GateExpectationStatus {
    fn is_blocking(&self) -> bool {
        matches!(self, GateExpectationStatus::Regressed)
    }
}

#[cfg(test)]
pub(crate) struct GateCaches<'a> {
    pub(crate) history: &'a mut HistoryCache,
    pub(crate) visible_tree_oid: &'a mut VisibleTreeOidCache,
}

fn write_gate_failure_event(event: GateFailureEvent) -> Result<(), String> {
    match event {
        GateFailureEvent::Regressed => {
            // Gate output stays generic by canon: even expectation-related
            // failures are reported without expectation IDs or per-expectation
            // lines. `canon check` is the command that prints individual
            // expectation records.
            write_stderr_line("canon gate: staged changes regress cached canon results")?;
            write_stderr_line(gate_regression_advice())
        }
    }
}

pub(crate) fn gate_regression_advice() -> &'static str {
    "▷ Fix staged regressions and run `canon check` again!"
}

pub(crate) fn gate_error_advice() -> &'static str {
    "▷ Fix the gate error and run `canon check` again!"
}

#[derive(Debug, Clone)]
pub(crate) enum GateCacheResult {
    Pass,
    Fail,
    Missing,
}

pub(crate) fn gate_cached_result_for_tree(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    tree: GateComparisonTree,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<GateCacheResult, String> {
    gate_cache_result_for_tree_at(
        root,
        agent,
        expectation,
        tree,
        history_cache,
        visible_tree_oid_cache,
        unix_timestamp()?,
    )
}

fn gate_cache_result_for_tree_at(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    tree: GateComparisonTree,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<GateCacheResult, String> {
    let same_tree = latest_history_record_matching_visible_tree_oid(
        root,
        expectation,
        history_cache,
        |scope| match tree {
            GateComparisonTree::StagedIndex => visible_tree_oid_cache
                .staged_visible_tree_oid(root, agent, scope)
                .map(Some),
            GateComparisonTree::Head => {
                visible_tree_oid_cache.gate_head_tree_fingerprint(root, agent, scope)
            }
        },
    )?;
    let cooldown = cooldown_history_record(root, agent, expectation, history_cache, now)?;
    let record = cached_history_record(same_tree, cooldown).map(|hit| match hit {
        CachedHistoryRecord::SameTree(record) | CachedHistoryRecord::Cooldown(record) => record,
    });
    match record {
        Some(record) if record.passed() => Ok(GateCacheResult::Pass),
        Some(_) => Ok(GateCacheResult::Fail),
        None => Ok(GateCacheResult::Missing),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum GateComparisonTree {
    StagedIndex,
    Head,
}
