use crate::check_cache::final_selected_after_current_pass_cache;
use crate::check_preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
use crate::check_selection::{
    expectation_identities, final_selected_expectations, select_expectations_with_identities,
    ExpectationIdentity,
};
use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::history::HistoryCache;
use crate::history_reuse::latest_history_record_matching_hash;
use crate::logging::render_check_log_record;
use crate::output::{write_stderr, write_stderr_line};
use crate::repo_inspection::RepoInspectionCache;
use crate::scope_hash::ScopeHashCache;
use crate::time::unix_timestamp;
use crate::CHECK_PATH;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    // CLI validation happens before the gate pass/fail decision. These
    // unsupported-option errors are usage errors, not `GateFailed` outcomes.
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".into());
    }
    match gate_project_change(root)? {
        GateProjectChange::MixedCanonAndNonCanon => {
            write_stderr_line(
                "canon gate: .canon/** changes must not be mixed with non-.canon changes",
            )?;
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
                args,
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
    Regressed(Box<CheckRecord>),
    Missing(SelectedExpectation),
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
    args: &[OsString],
    caches: GateCaches<'_>,
    now: u64,
    mut emit_failure: impl FnMut(GateFailureEvent) -> Result<(), String>,
) -> Result<bool, String> {
    // Gate resolves the same final selected set as `canon check` for command
    // selection consistency. Its pass/fail loop is intentionally separate: the
    // gate spec compares selector-expanded candidates for HEAD-to-staged
    // regressions, including candidates later deselected by cooldown or exact
    // current-cache passes.
    let _final_selected = select_expectations_for_gate(
        root,
        config,
        identities,
        args,
        caches.history,
        caches.scope_hash,
        now,
    )?;
    let selector_expanded = select_expectations_with_identities(config, identities, args)?;
    for expectation in &selector_expanded {
        let previous = exact_gate_cache_result_for_tree(
            root,
            &config.agent,
            expectation,
            GateComparisonTree::Head,
            caches.history,
            caches.scope_hash,
        )?;
        let current = exact_gate_cache_result_for_tree(
            root,
            &config.agent,
            expectation,
            GateComparisonTree::StagedIndex,
            caches.history,
            caches.scope_hash,
        )?;
        match (previous, current) {
            (GateCacheResult::Pass, GateCacheResult::Pass) => {}
            (GateCacheResult::Pass, GateCacheResult::Fail(record)) => {
                emit_failure(GateFailureEvent::Regressed(record))?;
                return Ok(false);
            }
            (GateCacheResult::Pass, GateCacheResult::Missing) => {
                emit_failure(GateFailureEvent::Missing(expectation.clone()))?;
                emit_failure(GateFailureEvent::MissingComplete {
                    has_regressions: false,
                })?;
                return Ok(false);
            }
            _ => {}
        }
    }
    Ok(true)
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
        GateFailureEvent::Regressed(record) => {
            write_stderr_line("canon gate: expectations regressed to cached fail:")?;
            let line = render_check_log_record(&record).map_err(|err| err.to_string())?;
            write_stderr(&line)
        }
        GateFailureEvent::Missing(expectation) => {
            if !*wrote_missing_header {
                write_stderr_line("canon gate: missing cached answers for expectations:")?;
                *wrote_missing_header = true;
            }
            write_stderr_line(&format!("canon gate: - {}", expectation.display_id))
        }
        GateFailureEvent::MissingComplete { has_regressions } => {
            if let Some(advice) = gate_missing_cache_advice(has_regressions) {
                write_stderr_line(advice)?;
            }
            Ok(())
        }
    }
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

pub(crate) fn select_expectations_for_gate(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    args: &[OsString],
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
    now: u64,
) -> Result<Vec<SelectedExpectation>, String> {
    let selected = select_expectations_with_identities(config, identities, args)?;
    final_selected_expectations_for_gate(
        root,
        config,
        selected,
        history_cache,
        scope_hash_cache,
        now,
    )
}

fn final_selected_expectations_for_gate(
    root: &Path,
    config: &CheckConfig,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
    now: u64,
) -> Result<Vec<SelectedExpectation>, String> {
    let selected = final_selected_expectations(root, &config.agent, selected, history_cache, now)
        .map(|selection| selection.selected)
        .map_err(|err| err.error)?;
    final_selected_after_current_pass_cache(
        root,
        &config.agent,
        selected,
        history_cache,
        scope_hash_cache,
    )
    .map(|selection| selection.selected)
}

#[derive(Debug, Clone)]
pub(crate) enum GateCacheResult {
    Pass,
    Fail(Box<CheckRecord>),
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
                    scope_hash_cache.gate_head_tree_fingerprint(root, scope)
                }
            },
        )?;
    match record {
        Some(record) if record.passed() => Ok(GateCacheResult::Pass),
        Some(record) => Ok(GateCacheResult::Fail(Box::new(record))),
        None => Ok(GateCacheResult::Missing),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum GateComparisonTree {
    StagedIndex,
    Head,
}
