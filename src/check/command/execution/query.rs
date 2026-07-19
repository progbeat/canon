use crate::app::LazyAppServerRunner;
use crate::check::command::output::{
    finish_query_output, start_query_report_output, SharedCheckOutput,
};
use crate::check::command::{
    collect_check_token_usage, prepare_git_backed_check_execution,
    resolve_git_backed_check_tree_context, GitBackedCheckResources,
    PrepareGitBackedCheckExecutionOptions,
};
use crate::check::core::ParsedAnswer;
use crate::check::interrogation::query::{
    query_human_review_reason, run_query_with_runner, QueryExpectationContext, QueryRequest,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::{CheckRunCaches, ResolvedExpectation};
use crate::config_types::{CheckConfig, Expectation, ExpectationTo};
use crate::evaluator::EvaluatorRunner;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::token_usage_types::TokenUsage;
use std::path::Path;

pub(crate) struct CheckQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) against_tree: Option<&'a TreeSource>,
    pub(crate) in_place: bool,
    pub(crate) diagnostic_log: Option<DiagnosticLogWriter>,
    pub(crate) check_caches: &'a mut CheckRunCaches,
    pub(crate) token_usage: &'a mut Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckQueryError {
    Command(String),
    Evaluator(String),
    Output(String),
    ReviewRequired(&'static str),
    DiagnosticLog {
        primary: Option<Box<CheckQueryError>>,
        error: String,
    },
}

impl std::fmt::Display for CheckQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckQueryError::Command(message)
            | CheckQueryError::Evaluator(message)
            | CheckQueryError::Output(message) => formatter.write_str(message),
            CheckQueryError::ReviewRequired(reason) => {
                write!(formatter, "query requires human review: {reason}")
            }
            CheckQueryError::DiagnosticLog {
                primary: Some(primary),
                error,
            } => {
                write!(
                    formatter,
                    "{primary}; also failed to write query runtime log: {error}"
                )
            }
            CheckQueryError::DiagnosticLog {
                primary: None,
                error,
            } => write!(formatter, "failed to write query runtime log: {error}"),
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
        tree_source,
        against_tree,
        in_place,
        diagnostic_log,
        check_caches,
        token_usage,
    } = command;
    let mut diagnostic_log = diagnostic_log;
    if let Some(writer) = diagnostic_log.as_mut() {
        // [7N,Ky] Ask event production remains unconditional, while persistent
        // storage failures are deferred until evaluation, public output, usage
        // collection, and lifecycle finishing have all been attempted.
        writer.defer_write_errors();
        write_query_lifecycle_start_event(writer).map_err(|err| err.to_string())?;
    }
    let result = run_started_check_query_command(StartedCheckQueryCommand {
        root,
        config,
        tree_source,
        against_tree,
        in_place,
        diagnostic_log: diagnostic_log.as_mut(),
        check_caches,
        token_usage,
    });
    let finish_error = result.as_ref().err().map(ToString::to_string);
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_finish_event(writer, finish_error.as_deref())
            .map_err(|err| err.to_string())?;
    }
    let diagnostic_log_error = diagnostic_log
        .as_mut()
        .and_then(|writer| writer.finish_deferred_writes().err());
    finish_query_command(result, diagnostic_log_error)
}

fn finish_query_command(
    result: Result<(), CheckQueryError>,
    diagnostic_log_error: Option<String>,
) -> Result<(), CheckQueryError> {
    match diagnostic_log_error {
        Some(error) => Err(CheckQueryError::DiagnosticLog {
            primary: result.err().map(Box::new),
            error,
        }),
        None => result,
    }
}

struct StartedCheckQueryCommand<'a, 'b> {
    root: &'a Path,
    config: &'a CheckConfig,
    tree_source: Option<&'a TreeSource>,
    against_tree: Option<&'a TreeSource>,
    in_place: bool,
    diagnostic_log: Option<&'b mut DiagnosticLogWriter>,
    check_caches: &'a mut CheckRunCaches,
    token_usage: &'a mut Option<TokenUsage>,
}

fn run_started_check_query_command(
    command: StartedCheckQueryCommand<'_, '_>,
) -> Result<(), CheckQueryError> {
    let StartedCheckQueryCommand {
        root,
        config,
        tree_source,
        against_tree,
        in_place,
        mut diagnostic_log,
        check_caches,
        token_usage,
    } = command;
    let mut in_place_runner;
    let (runtime, runner): (CheckRuntime<'_>, &mut LazyAppServerRunner) = if in_place {
        // [3i5] Ask uses a state-free, read-only evaluator runner. Runtime logs
        // are owned by the command boundary and never enter this runner.
        in_place_runner = LazyAppServerRunner::new_in_place(
            root,
            crate::check::config::validation::check_config_loads_plugins(config),
            &config.agent,
            false,
        )?;
        (
            CheckRuntime::in_place_temporary_query(root, config),
            &mut in_place_runner,
        )
    } else {
        let tree_source = tree_source.ok_or_else(|| "missing query tree source".to_string())?;
        let against_tree = against_tree.ok_or_else(|| "missing query against tree".to_string())?;
        // [3i5] Git-backed ask also keeps prompt objects and materialized trees
        // invocation-local, with no app-server or xpec history sink.
        let resources = GitBackedCheckResources::temporary_query(root)?;
        let tree_context = resolve_git_backed_check_tree_context(
            root,
            tree_source,
            against_tree,
            &mut check_caches.visible_tree_oid,
            &resources,
        )?;
        let mut execution = prepare_git_backed_check_execution(
            root,
            config,
            PrepareGitBackedCheckExecutionOptions {
                tree_source,
                tree_context,
                no_sandbox: false,
                resources,
            },
        )?;
        let runtime = CheckRuntime::materialized_without_persistent_history(
            root,
            &execution.staged_view,
            &execution.tree_source,
            execution.tree_context.clone(),
            config,
            false,
        );
        let runner = &mut execution.runner;
        return run_prepared_query(
            root,
            runtime,
            runner,
            config,
            &mut diagnostic_log,
            check_caches,
            token_usage,
        );
    };
    run_prepared_query(
        root,
        runtime,
        runner,
        config,
        &mut diagnostic_log,
        check_caches,
        token_usage,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_prepared_query(
    _root: &Path,
    runtime: CheckRuntime<'_>,
    runner: &mut LazyAppServerRunner,
    config: &CheckConfig,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
    token_usage: &mut Option<TokenUsage>,
) -> Result<(), CheckQueryError> {
    // xpec: 3i5
    assert!(
        runtime.persistent_check_state_root().is_none(),
        "prepared ask runtime must not expose persistent xpec history"
    );
    // The ask-only config contains exactly one temporary xpec whose explicit
    // to/q/a fields and selected preset defaults were resolved together during
    // raw expansion. Configured check expectations are not part of this query.
    let enforced_scope = runtime
        .scope_without_reusable_q_scope_history()
        .expect("a temporary query has no reusable q-scope history");
    let [configured_temporary_expectation] = config.expectations.as_slice() else {
        return Err("ask config must contain exactly one temporary expectation"
            .to_string()
            .into());
    };
    let temporary_expectation = temporary_query_expectation(configured_temporary_expectation);
    let expectation = QueryExpectationContext {
        expectation: &temporary_expectation,
    };
    // This is the `canon ask` evaluator boundary: every prepared ask creates a
    // temporary resultless xpec and sends it through the same interrogation path
    // as check evaluation. There is no cache hit or last-result shortcut here.
    let mut interrogation_run_state =
        InterrogationRunState::new(runtime.disable_session_isolation())?;
    let shared_output = SharedCheckOutput::stdout();
    let started_report = start_query_report_output(shared_output);
    let progress = started_report.progress();
    runner.set_progress_reporter(Some(progress.clone()));
    let result = run_query_with_runner(
        &runtime,
        QueryRequest {
            question: &temporary_expectation.question,
            enforced_scope: &enforced_scope,
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
            return match finish_query_output_and_collect_usage(
                started_report,
                &answer,
                runner,
                token_usage,
            ) {
                Ok(()) => Err(CheckQueryError::Evaluator(err)),
                Err(output_err) => Err(output_err),
            };
        }
    };
    finish_query_output_and_collect_usage(started_report, &result.answer, runner, token_usage)?;
    if let Some(reason) = query_human_review_reason(&result) {
        return Err(CheckQueryError::ReviewRequired(reason));
    }
    Ok(())
}

fn query_failure_answer(error: &str) -> ParsedAnswer {
    ParsedAnswer::error(error.to_string(), error.to_string())
}

fn finish_query_output_and_collect_usage(
    started_report: crate::check::command::output::StartedExpectationReportOutput,
    answer: &ParsedAnswer,
    runner: &mut LazyAppServerRunner,
    token_usage: &mut Option<TokenUsage>,
) -> Result<(), CheckQueryError> {
    // Collect usage even when stdout finishing fails. The outer ask command
    // boundary prints it after this result and all lifecycle cleanup.
    let output_result = finish_query_output(started_report, answer);
    *token_usage = collect_check_token_usage(runner);
    output_result.map_err(CheckQueryError::Output)
}

fn temporary_query_expectation(expectation: &Expectation) -> ResolvedExpectation {
    ResolvedExpectation {
        number: 0,
        id: String::new(),
        display_id: "q".to_string(),
        // [3i5] These fields belong to the `canon ask` command, not its
        // selected preset. Preset resolution supplies the remaining context.
        to: ExpectationTo::Agent,
        rank: expectation.rank,
        question: expectation.q.clone(),
        expected_answer: String::new(),
        question_context: expectation.question_context.clone(),
        diff_from: expectation.diff_from.clone(),
        target: expectation.target.clone(),
        question_answer_only: expectation.question_answer_only,
        agent: expectation.agent.clone(),
        cooldown: expectation.cooldown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{AgentConfig, ExpectationTarget};

    #[test] // xpec: 3i5,Ky,nK,LA
    fn temporary_query_expectation_forces_ask_owned_fields() {
        let agent = AgentConfig::implementation_default();
        let configured = Expectation {
            to: ExpectationTo::Caller,
            rank: 4,
            q: "Does ask work?".to_string(),
            a: "yes".to_string(),
            question_context: "Use selected preset context.".to_string(),
            diff_from: "HEAD~1".to_string(),
            target: Some(ExpectationTarget::Diff),
            question_answer_only: false,
            agent,
            cooldown: None,
            in_place_compatibility: Default::default(),
        };
        let expectation = temporary_query_expectation(&configured);

        assert_eq!(expectation.question, "Does ask work?");
        assert_eq!(expectation.expected_answer, "");
        assert_eq!(expectation.to, ExpectationTo::Agent);
        assert_eq!(expectation.display_id, "q");
        assert!(expectation.id.is_empty());
        assert_eq!(expectation.question_context, "Use selected preset context.");
        assert_eq!(expectation.diff_from, "HEAD~1");
        assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
    }

    #[test] // xpec: Ky
    fn query_failure_answer_reports_runtime_error_as_error_text() {
        let answer = query_failure_answer("transport failed");

        assert_eq!(answer.error.as_deref(), Some("transport failed"));
        assert_eq!(answer.evidence.as_deref(), Some("transport failed"));
    }

    #[test] // xpec: 7N,Ky
    fn deferred_query_log_error_preserves_primary_result() {
        let result = finish_query_command(
            Err(CheckQueryError::ReviewRequired("invalid question")),
            Some("sink failed".to_string()),
        )
        .unwrap_err();

        assert_eq!(
            result,
            CheckQueryError::DiagnosticLog {
                primary: Some(Box::new(CheckQueryError::ReviewRequired(
                    "invalid question"
                ))),
                error: "sink failed".to_string(),
            }
        );
    }
}
