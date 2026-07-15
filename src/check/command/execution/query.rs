use crate::app::LazyAppServerRunner;
use crate::check::command::output::{
    finish_query_output, start_query_report_output, SharedCheckOutput,
};
use crate::check::command::{
    collect_check_token_usage, prepare_git_backed_check_execution, print_token_usage_summary,
    GitBackedCheckStorage, PrepareGitBackedCheckExecutionOptions,
};
use crate::check::config::validation::validate_in_place_global_config;
use crate::check::core::ParsedAnswer;
use crate::check::interrogation::query::{
    query_human_review_reason, run_query_with_runner, QueryExpectationContext, QueryRequest,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::{CheckRunCaches, ResolvedExpectation};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckQueryError {
    Command(String),
    Evaluator(String),
    Output(String),
    TokenUsage(String),
    ReviewRequired(&'static str),
}

impl std::fmt::Display for CheckQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckQueryError::Command(message)
            | CheckQueryError::Evaluator(message)
            | CheckQueryError::Output(message)
            | CheckQueryError::TokenUsage(message) => formatter.write_str(message),
            CheckQueryError::ReviewRequired(reason) => {
                write!(formatter, "query requires human review: {reason}")
            }
        }
    }
}

impl From<String> for CheckQueryError {
    fn from(message: String) -> CheckQueryError {
        CheckQueryError::Command(message)
    }
}

pub(crate) fn run_check_query_command(
    command: CheckQueryCommand<'_>,
) -> Result<(), CheckQueryError> {
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
    let finish_error = result.as_ref().err().map(ToString::to_string);
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_finish_event(writer, finish_error.as_deref())
            .map_err(|err| err.to_string())?;
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
) -> Result<(), CheckQueryError> {
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
    // Scope sanitization is command validation. A prepared ask starts only
    // after this succeeds, then sends the temporary xpec to the evaluator.
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
        let mut execution = prepare_git_backed_check_execution(
            root,
            config,
            PrepareGitBackedCheckExecutionOptions {
                tree_source,
                against_tree,
                no_sandbox,
                storage: GitBackedCheckStorage::InvocationLocal,
            },
            &mut check_caches.visible_tree_oid,
        )?;
        let runtime = CheckRuntime::materialized_without_persistent_history(
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
) -> Result<(), CheckQueryError> {
    // `config` is the ask-only config assembled by run.rs: command `--preset`
    // can only choose the default agent during raw expansion, and configured
    // check expectations/hooks are not part of this query. In-place query
    // validation therefore covers the global agent settings used below.
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
    // This is the `canon ask` evaluator boundary: every prepared ask creates a
    // temporary resultless xpec and sends it through the same interrogation path
    // as check evaluation. There is no cache hit or last-result shortcut here.
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
            let answer = query_failure_answer(&err);
            return match finish_query_output_and_print_usage(started_report, &answer, runner) {
                Ok(()) => Err(CheckQueryError::Evaluator(err)),
                Err(output_err) => Err(output_err),
            };
        }
    };
    finish_query_output_and_print_usage(started_report, &result.answer, runner)?;
    if let Some(reason) = query_human_review_reason(&result) {
        return Err(CheckQueryError::ReviewRequired(reason));
    }
    Ok(())
}

fn query_failure_answer(error: &str) -> ParsedAnswer {
    ParsedAnswer::error(error.to_string(), error.to_string())
}

fn finish_query_output_and_print_usage(
    started_report: crate::check::command::output::StartedExpectationReportOutput,
    answer: &ParsedAnswer,
    runner: &mut LazyAppServerRunner,
) -> Result<(), CheckQueryError> {
    // Attempt both public output surfaces before returning either error. This
    // preserves the query token-usage stderr line even when stdout finishing
    // reports a write/flush failure.
    let output_result = finish_query_output(started_report, answer);
    let usage_result = print_query_token_usage(runner);
    match (output_result, usage_result) {
        (Err(err), _) => Err(CheckQueryError::Output(err)),
        (Ok(()), Err(err)) => Err(CheckQueryError::TokenUsage(err)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn print_query_token_usage(runner: &mut LazyAppServerRunner) -> Result<(), String> {
    let usage = collect_check_token_usage(runner);
    print_token_usage_summary(usage)
}

fn query_enforced_scope(query_scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope(query_scope).map_err(|err| format!("--scope: {}", err))
}

fn temporary_query_expectation(question: &str, agent: &AgentConfig) -> ResolvedExpectation {
    ResolvedExpectation {
        number: 0,
        id: String::new(),
        display_id: "q".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: question.to_string(),
        expected_answer: String::new(),
        question_context: String::new(),
        diff_from: DEFAULT_DIFF_FROM.to_string(),
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

    #[test] // xpec: f
    fn query_failure_answer_reports_runtime_error_as_error_text() {
        let answer = query_failure_answer("transport failed");

        assert_eq!(answer.error.as_deref(), Some("transport failed"));
        assert_eq!(answer.evidence, "transport failed");
    }
}
