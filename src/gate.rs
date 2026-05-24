use crate::check_preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
use crate::check_selection::{
    expectation_identities, select_expectations_with_identities, ExpectationIdentity,
};
use crate::check_types::SelectedExpectation;
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::history::HistoryCache;
use crate::history_reuse::latest_history_record_matching_hash;
use crate::output::write_stderr_line;
use crate::repo_inspection::RepoInspectionCache;
use crate::scope_hash::ScopeHashCache;
use crate::time::unix_timestamp;
use crate::CHECK_PATH;
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
    match gate_project_change(root)? {
        GateProjectChange::MixedCanonAndNonCanon => {
            write_stderr_line(
                "canon gate: .canon/** changes must not be mixed with non-.canon changes",
            )?;
            write_stderr_line("▷ Ask human to handle .canon/ changes.")?;
            Err(CommandError::GateFailed)
        }
        GateProjectChange::CanonOnly => Ok(()),
        GateProjectChange::Other => {
            let mut repo_cache = RepoInspectionCache::new();
            let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH))?;
            let mut scope_hash_cache = ScopeHashCache::new();
            let mut history_cache = HistoryCache::new();
            let now = unix_timestamp()?;
            let identities = expectation_identities(&config)?;
            let mut wrote_missing_header = false;
            let passed = gate_pass_with_config(
                root,
                &config,
                &identities,
                GateCaches {
                    history: &mut history_cache,
                    scope_hash: &mut scope_hash_cache,
                },
                now,
                |event| write_gate_failure_event(event, &mut wrote_missing_header),
            )?;
            if passed {
                Ok(())
            } else {
                Err(CommandError::GateFailed)
            }
        }
    }
}

pub(crate) enum GateFailureEvent {
    Regressed,
    Missing,
    MissingComplete { has_regressions: bool },
}

enum GateProjectChange {
    MixedCanonAndNonCanon,
    CanonOnly,
    Other,
}

fn gate_project_change(root: &Path) -> Result<GateProjectChange, String> {
    let changed_paths = staged_changed_path_bytes(root)?;
    let has_canon_change = changed_paths
        .iter()
        .any(|path| is_canon_project_path_bytes(path));
    if has_canon_change && !is_canon_only_staged_change_bytes(&changed_paths) {
        return Ok(GateProjectChange::MixedCanonAndNonCanon);
    }
    if has_canon_change {
        return Ok(GateProjectChange::CanonOnly);
    }
    Ok(GateProjectChange::Other)
}

pub(crate) fn gate_pass_with_config(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    mut caches: GateCaches<'_>,
    _now: u64,
    emit_failure: impl FnMut(GateFailureEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let selected_expectations = select_expectations_with_identities(config, identities, &[])?;
    // The gate pseudocode is the raw comparison loop over every no-selector
    // expectation. Cooldown is only a `canon check` work-saving filter; the
    // pre-commit gate still compares cached HEAD and staged results for the
    // selected expectations.
    gate_selected_expectations(
        root,
        &config.agent,
        &selected_expectations,
        &mut caches,
        emit_failure,
    )
}

pub(crate) fn gate_regression_count_with_config(
    root: &Path,
    config: &CheckConfig,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    // This is the gate spec's expectation comparison as a count: HEAD pass
    // followed by staged non-pass or missing cache.
    let identities = expectation_identities(config)?;
    let selected_expectations = select_expectations_with_identities(config, &identities, &[])?;
    selected_expectations
        .iter()
        .map(|expectation| {
            Ok(gate_expectation_status(
                root,
                &config.agent,
                expectation,
                history_cache,
                scope_hash_cache,
            )?
            .is_blocking() as usize)
        })
        .sum()
}

fn gate_selected_expectations(
    root: &Path,
    agent: &AgentConfig,
    selected_expectations: &[SelectedExpectation],
    caches: &mut GateCaches<'_>,
    mut emit_failure: impl FnMut(GateFailureEvent) -> Result<(), String>,
) -> Result<bool, String> {
    // These statuses are the only expectation-related way `canon gate` can
    // fail. Non-OK check results can remain non-blocking here when they did not
    // regress from a HEAD pass.
    // `gate_regression_count_with_config` counts these same statuses for
    // `canon check`'s `num_regressions`; if that count is zero, this loop has
    // no expectation-related failure branch to take.
    for expectation in selected_expectations {
        match gate_expectation_status(root, agent, expectation, caches.history, caches.scope_hash)?
        {
            GateExpectationStatus::PassedOrNonBlocking => {}
            GateExpectationStatus::Regressed => {
                emit_failure(GateFailureEvent::Regressed)?;
                return Ok(false);
            }
            GateExpectationStatus::MissingAfterHeadPass => {
                emit_failure(GateFailureEvent::Missing)?;
                emit_failure(GateFailureEvent::MissingComplete {
                    has_regressions: false,
                })?;
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
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<GateExpectationStatus, String> {
    let previous = exact_gate_cache_result_for_tree(
        root,
        agent,
        expectation,
        GateComparisonTree::Head,
        history_cache,
        scope_hash_cache,
    )?;
    let current = exact_gate_cache_result_for_tree(
        root,
        agent,
        expectation,
        GateComparisonTree::StagedIndex,
        history_cache,
        scope_hash_cache,
    )?;
    Ok(match (previous, current) {
        (GateCacheResult::Pass, GateCacheResult::Fail) => GateExpectationStatus::Regressed,
        (GateCacheResult::Pass, GateCacheResult::Missing) => {
            GateExpectationStatus::MissingAfterHeadPass
        }
        _ => GateExpectationStatus::PassedOrNonBlocking,
    })
}

enum GateExpectationStatus {
    PassedOrNonBlocking,
    Regressed,
    MissingAfterHeadPass,
}

impl GateExpectationStatus {
    fn is_blocking(&self) -> bool {
        matches!(
            self,
            GateExpectationStatus::Regressed | GateExpectationStatus::MissingAfterHeadPass
        )
    }
}

pub(crate) struct GateCaches<'a> {
    pub(crate) history: &'a mut HistoryCache,
    pub(crate) scope_hash: &'a mut ScopeHashCache,
}

fn write_gate_failure_event(
    event: GateFailureEvent,
    wrote_missing_header: &mut bool,
) -> Result<(), String> {
    match event {
        GateFailureEvent::Regressed => {
            write_stderr_line("canon gate: staged changes regress cached canon results")?;
            write_stderr_line(gate_regression_advice())
        }
        GateFailureEvent::Missing => {
            if !*wrote_missing_header {
                write_stderr_line("canon gate: missing cached canon answers for staged changes")?;
                *wrote_missing_header = true;
            }
            Ok(())
        }
        GateFailureEvent::MissingComplete { has_regressions } => {
            if let Some(advice) = gate_missing_cache_advice(has_regressions) {
                write_stderr_line(advice)?;
            }
            Ok(())
        }
    }
}

pub(crate) fn gate_regression_advice() -> &'static str {
    "▷ Fix staged regressions and run `canon check` again!"
}

pub(crate) fn gate_missing_cache_advice(has_regressions: bool) -> Option<&'static str> {
    // Regressions are the blocking action. When regressions and missing cache
    // records coexist, do not spend tokens filling unrelated missing records.
    if has_regressions {
        Some("canon gate: fix staged regressions before filling missing cache")
    } else {
        Some("canon gate: run `canon check` before committing")
    }
}

#[derive(Debug, Clone)]
pub(crate) enum GateCacheResult {
    Pass,
    Fail,
    Missing,
}

pub(crate) fn exact_gate_cache_result_for_tree(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    tree: GateComparisonTree,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<GateCacheResult, String> {
    let record =
        latest_history_record_matching_hash(
            root,
            expectation,
            history_cache,
            |scope| match tree {
                GateComparisonTree::StagedIndex => scope_hash_cache
                    .staged_scope_hash(root, agent, scope)
                    .map(Some),
                GateComparisonTree::Head => {
                    scope_hash_cache.gate_head_tree_fingerprint(root, agent, scope)
                }
            },
        )?;
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
