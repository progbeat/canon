use crate::check::command::output::write_query_output;
use crate::check::command::reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
};
use crate::check::command::{prepare_check_execution, PrepareCheckExecutionOptions};
use crate::check::core::types::SelectedExpectation;
use crate::check::interrogation::interrogation_state::{
    initial_visible_scope_for_expectation, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::query::run_query_with_runner;
use crate::check::run::lazy_reset::clear_active_lazy_full_scope_reset_ids;
use crate::check::run::selection::{selected_expectation_at, ExpectationIdentity};
use crate::config_types::{CheckConfig, Expectation};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use serde_json::json;
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

pub(crate) struct CheckQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) identities: &'a [ExpectationIdentity],
    pub(crate) active_lazy_full_scope_reset_ids: &'a BTreeSet<String>,
    pub(crate) question: &'a str,
    pub(crate) query_scope: &'a [String],
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) no_sandbox: bool,
    pub(crate) diagnostic_log: DiagnosticLogWriter,
}

pub(crate) fn run_check_query_command(command: CheckQueryCommand<'_>) -> Result<(), String> {
    let CheckQueryCommand {
        root,
        config,
        identities,
        active_lazy_full_scope_reset_ids,
        question,
        query_scope,
        tree_source,
        no_sandbox,
        mut diagnostic_log,
    } = command;
    let no_applied_lazy_full_scope_reset_ids = BTreeSet::new();
    // `canon check -q` runs one ad-hoc query. When that query is exactly a
    // plain q/a expectation, reuse the expectation-mode initial q-scope so the
    // first evaluator input stays identical. An explicit `--scope` remains a
    // hard query boundary; query mode still verifies narrower reusable scopes.
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
    let matching_expectation = matching_q_a_only_expectation(config, identities, question)?;
    let enforced_scope = match query_enforced_scope(
        root,
        config,
        query_scope,
        matching_expectation.as_ref(),
        active_lazy_full_scope_reset_ids,
    ) {
        Ok(scope) => scope,
        Err(err) => {
            write_query_error_finish(
                root,
                &no_applied_lazy_full_scope_reset_ids,
                &mut diagnostic_log,
                &err,
            )?;
            return Err(err);
        }
    };
    let applied_lazy_full_scope_reset_ids = query_applied_lazy_full_scope_reset_ids(
        query_scope,
        matching_expectation.as_ref(),
        active_lazy_full_scope_reset_ids,
    );
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut execution = prepare_check_execution(
        root,
        config,
        &mut diagnostic_log,
        PrepareCheckExecutionOptions {
            tree_source,
            no_sandbox,
            query: true,
            errors_on_failure: 1,
        },
        &mut visible_tree_oid_cache,
    )?;
    let runtime = CheckRuntime::materialized(
        root,
        &execution.staged_view,
        &execution.tree_source,
        config,
        no_sandbox,
    );
    let mut interrogation_run_state = match InterrogationRunState::new(runtime.no_sandbox()) {
        Ok(state) => state,
        Err(err) => {
            write_query_error_finish(
                root,
                &applied_lazy_full_scope_reset_ids,
                &mut diagnostic_log,
                &err,
            )?;
            return Err(err);
        }
    };
    let result = run_query_with_runner(
        &runtime,
        question,
        matching_expectation
            .as_ref()
            .map(|expectation| expectation.a.as_str())
            .or_else(|| query_expected_answer(config, question)),
        &enforced_scope,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        &mut interrogation_run_state,
    );
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            let usage = collect_check_token_usage(&mut execution.runner)?;
            print_token_usage_summary(Some(usage))?;
            write_query_error_finish(
                root,
                &applied_lazy_full_scope_reset_ids,
                &mut diagnostic_log,
                &err,
            )?;
            return Err(err);
        }
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    // The query answer is the only public stdout piece eligible at this point;
    // `write_query_output` writes and flushes it before usage collection starts.
    if let Err(err) = write_query_output(&mut stdout, &result.answer) {
        write_query_error_finish(
            root,
            &applied_lazy_full_scope_reset_ids,
            &mut diagnostic_log,
            &err,
        )?;
        return Err(err);
    }
    let usage = match collect_check_token_usage(&mut execution.runner) {
        Ok(usage) => usage,
        Err(err) => {
            write_query_error_finish(
                root,
                &applied_lazy_full_scope_reset_ids,
                &mut diagnostic_log,
                &err,
            )?;
            return Err(err);
        }
    };
    // Query token usage is the next public stderr piece. It is not eligible
    // until pending app-server usage updates are drained above; once known,
    // `print_token_usage_summary` writes and flushes it immediately.
    print_token_usage_summary(Some(usage))?;
    write_query_finish(
        root,
        &applied_lazy_full_scope_reset_ids,
        &mut diagnostic_log,
        None,
    )
}

fn write_query_error_finish(
    root: &Path,
    applied_lazy_full_scope_reset_ids: &BTreeSet<String>,
    diagnostic_log: &mut DiagnosticLogWriter,
    err: &str,
) -> Result<(), String> {
    write_query_finish(
        root,
        applied_lazy_full_scope_reset_ids,
        diagnostic_log,
        Some(err),
    )
}

fn write_query_finish(
    root: &Path,
    applied_lazy_full_scope_reset_ids: &BTreeSet<String>,
    diagnostic_log: &mut DiagnosticLogWriter,
    err: Option<&str>,
) -> Result<(), String> {
    let mut finish_error = err.map(str::to_string);
    // A query can consume only the matching expectation's active reset, and
    // only when its implicit scope actually used that reset. Unrelated active
    // reset markers remain for a later expectation run.
    if let Err(reset_err) =
        clear_active_lazy_full_scope_reset_ids(root, applied_lazy_full_scope_reset_ids)
    {
        finish_error.get_or_insert(reset_err);
    }
    write_check_finish_event(diagnostic_log, true, finish_error.as_deref())
}

fn query_applied_lazy_full_scope_reset_ids(
    query_scope: &[String],
    matching_expectation: Option<&SelectedExpectation>,
    active_lazy_full_scope_reset_ids: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if !query_scope.is_empty() {
        return ids;
    }
    let Some(expectation) = matching_expectation else {
        return ids;
    };
    if active_lazy_full_scope_reset_ids.contains(&expectation.id) {
        ids.insert(expectation.id.clone());
    }
    ids
}

fn query_enforced_scope(
    root: &Path,
    config: &CheckConfig,
    query_scope: &[String],
    matching_expectation: Option<&SelectedExpectation>,
    active_lazy_full_scope_reset_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if !query_scope.is_empty() {
        return sanitize_scope(query_scope, &config.agent)
            .map_err(|err| format!("--scope: {}", err));
    }
    let Some(expectation) = matching_expectation else {
        return Ok(full_scope());
    };
    let mut history_cache = HistoryCache::default();
    initial_visible_scope_for_expectation(
        root,
        expectation,
        &mut history_cache,
        active_lazy_full_scope_reset_ids,
    )
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

fn matching_q_a_only_expectation(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    question: &str,
) -> Result<Option<SelectedExpectation>, String> {
    let mut matches = config
        .expectations
        .iter()
        .enumerate()
        .filter(|(_, expectation)| {
            expectation.q == question && expectation_defines_only_q_and_a(config, expectation)
        });
    let Some((index, _)) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Ok(None);
    }
    selected_expectation_at(config, identities, index, false).map(Some)
}

fn expectation_defines_only_q_and_a(config: &CheckConfig, expectation: &Expectation) -> bool {
    expectation.prompt_scope.is_empty()
        && expectation.cooldown.is_none()
        && expectation.thinking.is_none()
        && expectation.agent == config.agent
}
