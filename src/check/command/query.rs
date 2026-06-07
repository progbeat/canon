use crate::app::LazyAppServerRunner;
use crate::check::command::output::write_query_output;
use crate::check::command::reporting::{collect_check_token_usage, print_token_usage_summary};
use crate::check::command::{prepare_check_execution, PrepareCheckExecutionOptions};
use crate::check::core::types::QueryResult;
use crate::check::interrogation::query::run_query_with_runner;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::CheckRunCaches;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use std::io;
use std::path::Path;

pub(crate) struct CheckQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) question: &'a str,
    pub(crate) query_scope: &'a [String],
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) against_tree: &'a TreeSource,
    pub(crate) no_sandbox: bool,
    pub(crate) diagnostic_log: DiagnosticLogWriter,
    pub(crate) check_caches: &'a mut CheckRunCaches,
}

pub(crate) fn run_check_query_command(command: CheckQueryCommand<'_>) -> Result<(), String> {
    let CheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        tree_source,
        against_tree,
        no_sandbox,
        mut diagnostic_log,
        check_caches,
    } = command;
    write_query_lifecycle_start_event(&mut diagnostic_log).map_err(|err| err.to_string())?;
    let result = run_started_check_query_command(StartedCheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        tree_source,
        against_tree,
        no_sandbox,
        diagnostic_log: &mut diagnostic_log,
        check_caches,
    });
    let finish_error = result.as_ref().err().map(String::as_str);
    write_query_lifecycle_finish_event(&mut diagnostic_log, finish_error)
        .map_err(|err| err.to_string())?;
    result
}

struct StartedCheckQueryCommand<'a, 'b> {
    root: &'a Path,
    config: &'a CheckConfig,
    question: &'a str,
    query_scope: &'a [String],
    tree_source: &'a TreeSource,
    against_tree: &'a TreeSource,
    no_sandbox: bool,
    diagnostic_log: &'b mut DiagnosticLogWriter,
    check_caches: &'a mut CheckRunCaches,
}

fn run_started_check_query_command(
    command: StartedCheckQueryCommand<'_, '_>,
) -> Result<(), String> {
    let StartedCheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        tree_source,
        against_tree,
        no_sandbox,
        diagnostic_log,
        check_caches,
    } = command;
    let enforced_scope = query_enforced_scope(query_scope)?;
    let mut execution = prepare_check_execution(
        root,
        config,
        PrepareCheckExecutionOptions {
            tree_source,
            against_tree,
            no_sandbox,
        },
        &mut check_caches.visible_tree_oid,
    )?;
    let runtime = CheckRuntime::materialized(
        root,
        &execution.staged_view,
        &execution.tree_source,
        execution.tree_context.clone(),
        config,
        no_sandbox,
    );
    let mut interrogation_run_state = InterrogationRunState::new(runtime.no_sandbox())?;
    let result = run_query_with_runner(
        &runtime,
        question,
        &enforced_scope,
        &mut execution.runner,
        Some(diagnostic_log),
        &mut interrogation_run_state,
    );
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            print_query_token_usage(&mut execution.runner)?;
            return Err(err);
        }
    };
    write_successful_query_output(&result, &mut execution.runner)?;
    Ok(())
}

fn write_successful_query_output(
    result: &QueryResult,
    runner: &mut LazyAppServerRunner,
) -> Result<(), String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_query_output(&mut stdout, &result.answer)?;
    print_query_token_usage(runner)
}

fn print_query_token_usage(runner: &mut LazyAppServerRunner) -> Result<(), String> {
    let usage = collect_check_token_usage(runner)?;
    print_token_usage_summary(Some(usage))
}

fn query_enforced_scope(query_scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope(query_scope).map_err(|err| format!("--scope: {}", err))
}
