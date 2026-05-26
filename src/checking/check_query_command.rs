use crate::check_command::prepare_check_execution;
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_output::write_query_output;
use crate::check_query::run_query_with_runner;
use crate::check_reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
};
use crate::config_types::CheckConfig;
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use crate::visible_tree_oid::VisibleTreeOidCache;
use serde_json::json;
use std::io;
use std::path::Path;

pub(crate) fn run_check_query_command(
    root: &Path,
    config: &CheckConfig,
    question: &str,
    query_scope: &[String],
    mut diagnostic_log: DiagnosticLogWriter,
) -> Result<(), String> {
    // `canon check -q` runs one ad-hoc query, not the selected-expectation loop
    // that uses persisted history scope seeds. An explicit `--scope` is a hard
    // query boundary; query mode still verifies narrower reusable scopes.
    diagnostic_log
        .write_event(
            "info",
            "check.start",
            &[
                ("query", json!(true)),
                ("selected", json!(Vec::<usize>::new())),
            ],
        )
        .map_err(|err| err.to_string())?;
    let enforced_scope = match query_enforced_scope(config, query_scope) {
        Ok(scope) => scope,
        Err(err) => {
            write_query_error_finish(&mut diagnostic_log, &err)?;
            return Err(err);
        }
    };
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut execution = prepare_check_execution(
        root,
        config,
        &mut diagnostic_log,
        true,
        1,
        &mut visible_tree_oid_cache,
    )?;
    execution
        .staged_view
        .remove_evaluator_denied_paths(&config.agent)?;
    let runtime = CheckRuntime {
        root,
        snapshot_root: execution.staged_view.snapshot_root(),
        config,
    };
    let mut interrogation_state = InterrogationState::new();
    let result = run_query_with_runner(
        &runtime,
        question,
        query_expected_answer(config, question),
        &enforced_scope,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    );
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            let usage = collect_check_token_usage(&mut execution.runner)?;
            print_token_usage_summary(Some(usage))?;
            write_query_error_finish(&mut diagnostic_log, &err)?;
            return Err(err);
        }
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if let Err(err) = write_query_output(&mut stdout, &result.answer) {
        write_query_error_finish(&mut diagnostic_log, &err)?;
        return Err(err);
    }
    let usage = match collect_check_token_usage(&mut execution.runner) {
        Ok(usage) => usage,
        Err(err) => {
            write_query_error_finish(&mut diagnostic_log, &err)?;
            return Err(err);
        }
    };
    // Query token usage is the next public stderr piece. It is not computable
    // until pending app-server usage updates are drained above; once known,
    // `print_token_usage_summary` writes and flushes it immediately.
    print_token_usage_summary(Some(usage))?;
    // Query mode is ad-hoc and has no selected/cached expectation set; for the
    // lazy reset algorithm it is equivalent to `cached_expectations = []`.
    // Scheduled resets were already applied by `run_check_command`, but this
    // path must not plan a new reset from an empty query-only expectation set.
    write_check_finish_event(&mut diagnostic_log, true, None)
}

fn write_query_error_finish(
    diagnostic_log: &mut DiagnosticLogWriter,
    err: &str,
) -> Result<(), String> {
    write_check_finish_event(diagnostic_log, true, Some(err))
}

fn query_enforced_scope(
    config: &CheckConfig,
    query_scope: &[String],
) -> Result<Vec<String>, String> {
    if query_scope.is_empty() {
        Ok(full_scope())
    } else {
        sanitize_scope(query_scope, &config.agent).map_err(|err| format!("--scope: {}", err))
    }
}

fn query_expected_answer<'a>(config: &'a CheckConfig, question: &str) -> Option<&'a str> {
    let mut matches = config
        .expectations
        .iter()
        .filter(|expectation| expectation.q == question);
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first.a.as_str())
    }
}
