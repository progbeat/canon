use crate::check::{
    expectation_identities, is_canon_only_staged_change_bytes, is_canon_project_path_bytes,
    select_expectations_with_identities, staged_changed_path_bytes, SelectedExpectation,
    CHECK_PATH,
};
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::output::write_stderr_line;
use crate::repo_inspection::RepoInspectionCache;
use crate::time::unix_timestamp;
use crate::xpec_state::{
    cached_last_result_for_expectation, refresh_reused_same_tree_last_result, CachedResultStatus,
    XpecStateCache,
};
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
    let changed_paths = gate_result_or_failure(staged_changed_path_bytes(root))?;
    // `canon check` prints "Commit the staged changes NOW!" only when this
    // same HEAD-vs-staged regression count is zero. In that same-tree commit
    // case, remaining expectation failures are not gate failures; only a
    // staged regression from HEAD pass to staged fail blocks the hook.
    let num_regressions = gate_regression_count(root)?;
    match gate_decision(num_regressions, &changed_paths) {
        GateDecision::Pass => Ok(()),
        GateDecision::RegressionFailure => {
            write_gate_regression_failure()?;
            Err(CommandError::GateFailed)
        }
        GateDecision::MixedCanonChangeFailure => {
            write_mixed_canon_change_failure()?;
            Err(CommandError::GateFailed)
        }
    }
}

fn gate_regression_count(root: &Path) -> Result<usize, CommandError> {
    let mut repo_cache = RepoInspectionCache::new();
    let config =
        match repo_cache.load_check_config(root, Path::new(CHECK_PATH), &TreeSource::Staged) {
            Ok(config) => config,
            Err(_) => return Ok(0),
        };
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut xpec_state = XpecStateCache::default();
    gate_result_or_failure(gate_regression_count_with_config(
        root,
        &config,
        &mut xpec_state,
        &mut visible_tree_oid_cache,
    ))
}

enum GateDecision {
    Pass,
    RegressionFailure,
    MixedCanonChangeFailure,
}

fn gate_decision(num_regressions: usize, changed_paths: &[Vec<u8>]) -> GateDecision {
    if num_regressions > 0 {
        return GateDecision::RegressionFailure;
    }
    if has_mixed_canon_and_non_canon_changes(changed_paths) {
        return GateDecision::MixedCanonChangeFailure;
    }
    GateDecision::Pass
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

fn has_mixed_canon_and_non_canon_changes(changed_paths: &[Vec<u8>]) -> bool {
    let has_canon_change = changed_paths
        .iter()
        .any(|path| is_canon_project_path_bytes(path));
    has_canon_change && !is_canon_only_staged_change_bytes(changed_paths)
}

fn write_mixed_canon_change_failure() -> Result<(), String> {
    write_stderr_line("canon gate: .canon/** changes must not be mixed with non-.canon changes")?;
    write_stderr_line("▷ Ask human to handle .canon/ changes.")
}

pub(crate) fn gate_regression_count_with_config(
    root: &Path,
    config: &CheckConfig,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    let identities = expectation_identities(config)?;
    let selected_expectations = select_expectations_with_identities(config, &identities, &[])?;
    let now = unix_timestamp()?;
    gate_selected_regression_count(
        root,
        &config.agent,
        &selected_expectations,
        xpec_state,
        visible_tree_oid_cache,
        now,
    )
}

fn gate_selected_regression_count(
    root: &Path,
    agent: &AgentConfig,
    selected_expectations: &[SelectedExpectation],
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<usize, String> {
    // This is the gate spec's only expectation-related failure: HEAD pass
    // followed by staged fail. Non-OK check results remain non-blocking when
    // they did not regress from a HEAD pass.
    selected_expectations
        .iter()
        .map(|expectation| {
            Ok(gate_expectation_status(
                root,
                agent,
                expectation,
                xpec_state,
                visible_tree_oid_cache,
                now,
            )?
            .is_blocking() as usize)
        })
        .sum()
}

fn gate_expectation_status(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<GateExpectationStatus, String> {
    let previous = gate_cache_result_for_tree_at(
        root,
        agent,
        expectation,
        GateComparisonTree::Head,
        xpec_state,
        visible_tree_oid_cache,
        now,
    )?;
    let current = gate_cache_result_for_tree_at(
        root,
        agent,
        expectation,
        GateComparisonTree::StagedIndex,
        xpec_state,
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

fn write_gate_regression_failure() -> Result<(), String> {
    // Gate output stays generic by canon: even expectation-related failures
    // are reported without expectation IDs or per-expectation lines. `canon
    // check` is the command that prints individual expectation records.
    write_stderr_line("canon gate: staged changes regress cached canon results")?;
    write_stderr_line(gate_regression_advice())
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

fn gate_cache_result_for_tree_at(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    tree: GateComparisonTree,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
) -> Result<GateCacheResult, String> {
    let source = match tree {
        GateComparisonTree::StagedIndex => TreeSource::Staged,
        GateComparisonTree::Head => TreeSource::resolve_default_against_tree(
            root,
            crate::git::DEFAULT_AGAINST_TREE_ARG,
            false,
        )?,
    };
    let hit = cached_last_result_for_expectation(
        root,
        &source,
        expectation,
        xpec_state,
        visible_tree_oid_cache,
        now,
        true,
        true,
    )?;
    let hit = match hit {
        Some(hit) => Some(refresh_reused_same_tree_last_result(
            root,
            expectation,
            xpec_state,
            hit,
        )?),
        None => None,
    };
    match hit.map(|hit| hit.status) {
        Some(CachedResultStatus::Pass) => Ok(GateCacheResult::Pass),
        Some(CachedResultStatus::Fail) => Ok(GateCacheResult::Fail),
        None => Ok(GateCacheResult::Missing),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum GateComparisonTree {
    StagedIndex,
    Head,
}
