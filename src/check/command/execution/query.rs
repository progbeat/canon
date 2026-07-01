use super::in_place::validate_in_place_global_config;
use crate::app::LazyAppServerRunner;
use crate::check::command::output::{
    finish_query_output, start_query_report_output, SharedCheckOutput,
};
use crate::check::command::{
    collect_check_token_usage, prepare_check_execution, print_token_usage_summary,
    PrepareCheckExecutionOptions,
};
use crate::check::core::{ParsedAnswer, INTERNAL_ERROR_UNPARSABLE};
use crate::check::interrogation::query::{
    query_human_review_reason, run_query_with_runner, QueryExpectationContext, QueryRequest,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::{CheckRunCaches, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig, DEFAULT_DIFF_FROM};
use crate::evaluator::EvaluatorRunner;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use std::path::Path;

pub(crate) struct CheckQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) question: &'a str,
    pub(crate) query_scope: &'a [String],
    pub(crate) query_scope_provided: bool,
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) against_tree: Option<&'a TreeSource>,
    pub(crate) no_sandbox: bool,
    pub(crate) in_place: bool,
    pub(crate) diagnostic_log: Option<DiagnosticLogWriter>,
    pub(crate) check_caches: &'a mut CheckRunCaches,
}

pub(crate) fn run_check_query_command(command: CheckQueryCommand<'_>) -> Result<(), String> {
    let CheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        diagnostic_log,
        check_caches,
    } = command;
    let mut diagnostic_log = diagnostic_log;
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_start_event(writer).map_err(|err| err.to_string())?;
    }
    let result = run_started_check_query_command(StartedCheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        diagnostic_log: diagnostic_log.as_mut(),
        check_caches,
    });
    let finish_error = result.as_ref().err().map(String::as_str);
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_finish_event(writer, finish_error).map_err(|err| err.to_string())?;
    }
    result
}

struct StartedCheckQueryCommand<'a, 'b> {
    root: &'a Path,
    config: &'a CheckConfig,
    question: &'a str,
    query_scope: &'a [String],
    query_scope_provided: bool,
    tree_source: Option<&'a TreeSource>,
    against_tree: Option<&'a TreeSource>,
    no_sandbox: bool,
    in_place: bool,
    diagnostic_log: Option<&'b mut DiagnosticLogWriter>,
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
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        mut diagnostic_log,
        check_caches,
    } = command;
    let mut enforced_scope = query_enforced_scope(query_scope)?;
    let mut in_place_runner;
    let (runtime, runner): (CheckRuntime<'_>, &mut LazyAppServerRunner) = if in_place {
        in_place_runner = LazyAppServerRunner::new_in_place(
            root,
            crate::check::config::validation::check_config_loads_plugins(config),
            &config.agent,
            no_sandbox,
        )?;
        (
            CheckRuntime::in_place(root, config, no_sandbox),
            &mut in_place_runner,
        )
    } else {
        let tree_source = tree_source.ok_or_else(|| "missing query tree source".to_string())?;
        let against_tree = against_tree.ok_or_else(|| "missing query against tree".to_string())?;
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
        let runner = &mut execution.runner;
        return run_prepared_query(
            root,
            runtime,
            runner,
            config,
            question,
            query_scope_provided,
            &mut enforced_scope,
            &mut diagnostic_log,
            check_caches,
        );
    };
    run_prepared_query(
        root,
        runtime,
        runner,
        config,
        question,
        query_scope_provided,
        &mut enforced_scope,
        &mut diagnostic_log,
        check_caches,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_prepared_query(
    _root: &Path,
    runtime: CheckRuntime<'_>,
    runner: &mut LazyAppServerRunner,
    config: &CheckConfig,
    question: &str,
    _query_scope_provided: bool,
    enforced_scope: &mut Vec<String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
) -> Result<(), String> {
    // `config` is already expanded: command `--preset` can only choose the
    // default agent during raw expansion, before `canon ask` consumes resolved
    // expectation/config fields here.
    if runtime.is_in_place() {
        validate_in_place_global_config(&config.agent)?;
        *enforced_scope = runtime
            .fresh_scope_without_persistent_history()
            .expect("in-place query has no persistent q-scope");
    }
    let temporary_expectation = temporary_query_expectation(question, &config.agent);
    let expectation = QueryExpectationContext {
        expectation: &temporary_expectation,
    };
    let mut interrogation_run_state =
        InterrogationRunState::new(runtime.no_sandbox() || runtime.is_in_place())?;
    let shared_output = SharedCheckOutput::stdout();
    let started_report = start_query_report_output(shared_output);
    let progress = started_report.progress();
    runner.set_progress_reporter(Some(progress.clone()));
    let result = run_query_with_runner(
        &runtime,
        QueryRequest {
            question,
            enforced_scope,
            expectation,
            progress: Some(&progress),
        },
        runner,
        diagnostic_log.as_deref_mut(),
        &mut interrogation_run_state,
        check_caches,
    );
    runner.set_progress_reporter(None);
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            let answer = ParsedAnswer::error(INTERNAL_ERROR_UNPARSABLE.to_string(), err.clone());
            finish_query_output(started_report, &answer)?;
            print_query_token_usage(runner)?;
            return Err(err);
        }
    };
    finish_query_output(started_report, &result.answer)?;
    print_query_token_usage(runner)?;
    if let Some(reason) = query_human_review_reason(&result) {
        return Err(format!("query requires human review: {}", reason));
    }
    Ok(())
}

fn print_query_token_usage(runner: &mut LazyAppServerRunner) -> Result<(), String> {
    let usage = collect_check_token_usage(runner);
    print_token_usage_summary(usage)
}

fn query_enforced_scope(query_scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope(query_scope).map_err(|err| format!("--scope: {}", err))
}

fn temporary_query_expectation(question: &str, agent: &AgentConfig) -> SelectedExpectation {
    SelectedExpectation {
        number: 0,
        id: String::new(),
        display_id: "q".to_string(),
        question: question.to_string(),
        expected_answer: String::new(),
        question_context: String::new(),
        diff_from: DEFAULT_DIFF_FROM.to_string(),
        diff_from_configured: false,
        target: None,
        question_answer_only: true,
        agent: agent.clone(),
        cooldown: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;

    #[test]
    fn temporary_query_expectation_has_empty_expected_answer() {
        let agent = AgentConfig::implementation_default();
        let expectation = temporary_query_expectation("Does ask work?", &agent);

        assert_eq!(expectation.question, "Does ask work?");
        assert_eq!(expectation.expected_answer, "");
        assert_eq!(expectation.display_id, "q");
        assert!(expectation.id.is_empty());
    }
}
