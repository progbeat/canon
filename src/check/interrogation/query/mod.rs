use crate::check::core::{QueryResult, ResolvedExpectation};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::{
    write_query_result_event, write_query_review_required_event, InterrogationSession,
};
use crate::check::{
    run_temporary_expectation_interrogation, CheckRunCaches,
    TemporaryExpectationInterrogationContext,
};
use crate::evaluator::{EvaluatorProgress, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) expectation: QueryExpectationContext<'a>,
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryExpectationContext<'a> {
    pub(crate) expectation: &'a ResolvedExpectation,
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    interrogation_session: &mut InterrogationSession,
    caches: &mut CheckRunCaches,
) -> Result<QueryResult, String> {
    // Ask lifecycle start/finish events are emitted by the owning
    // `check::command::workflow::run::ask::query` boundary so they bracket
    // scope parsing and execution preparation as well as this evaluator turn.
    let mut diagnostic_log = diagnostic_log;
    let mut current_q_scope = query.enforced_scope.to_vec();
    let interrogation = run_temporary_expectation_interrogation(
        TemporaryExpectationInterrogationContext {
            runtime,
            runner,
            diagnostic_log: &mut diagnostic_log,
            caches,
            interrogation_session,
        },
        query.expectation.expectation,
        &mut current_q_scope,
        query.progress,
    )?;
    finish_query_result(
        query.question,
        &mut diagnostic_log,
        QueryResult {
            answer: interrogation.output.answer,
            diff_from: interrogation.output.diff_from,
            diff_from_tree_oid_abbrev: interrogation.output.diff_from_tree_oid_abbrev,
        },
    )
}

fn finish_query_result(
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    result: QueryResult,
) -> Result<QueryResult, String> {
    if let Some(reason) = result.human_review_reason() {
        write_query_review_required_event(question, diagnostic_log, &result.answer, reason)
            .map_err(|err| err.to_string())?;
        return Ok(result);
    }
    write_query_result_event(question, diagnostic_log, &result.answer)
        .map_err(|err| err.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test] // xpec: Eg
fn turn_timeout_retries_the_current_model_on_a_fresh_thread() {
    let root = tests::temp_root("turn-timeout-retry");
    let config = crate::config_types::CheckConfig {
        version: 1,
        agent: crate::config_types::AgentConfig::implementation_default(),
        expectations: Vec::new(),
    };
    let runtime = CheckRuntime::in_place(&root, &config, true);
    let expectation =
        temporary_query_expectation(&config, "Does timeout retry use a fresh thread?");
    let enforced_scope = crate::hash::full_scope();
    let request = QueryRequest {
        question: &expectation.question,
        enforced_scope: &enforced_scope,
        expectation: QueryExpectationContext {
            expectation: &expectation,
        },
        progress: None,
    };
    let mut runner = tests::FakeQueryRunner::with_turn_results(vec![
        Err(crate::evaluator::EvaluatorError::failure(
            crate::evaluator::EvaluatorFailureKind::TurnTimeout,
            "no-progress timeout",
        )),
        Ok(r#"{"q":{"answer":"yes","evidence":"fresh retry succeeded"}}"#.to_string()),
    ]);
    let mut caches = CheckRunCaches::new();
    let mut interrogation_session =
        InterrogationSession::new(true, caches.temporary_directory_allocator.clone()).unwrap();

    let result = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();

    assert_eq!(result.answer.observed, "yes");
    assert_eq!(runner.ask_thread_ids, ["thread-1", "thread-2"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
#[test] // xpec: qv
fn short_id_mismatch_after_a_valid_turn_starts_a_fresh_thread() {
    let root = tests::temp_root("short-id-retry");
    let config = crate::config_types::CheckConfig {
        version: 1,
        agent: crate::config_types::AgentConfig::implementation_default(),
        expectations: Vec::new(),
    };
    let runtime = CheckRuntime::in_place(&root, &config, true);
    let expectation = temporary_query_expectation(
        &config,
        "Does a short-ID mismatch retry use a fresh thread?",
    );
    let enforced_scope = crate::hash::full_scope();
    let request = QueryRequest {
        question: &expectation.question,
        enforced_scope: &enforced_scope,
        expectation: QueryExpectationContext {
            expectation: &expectation,
        },
        progress: None,
    };
    let mut runner = tests::FakeQueryRunner::with_turn_results(vec![
        Ok(r#"{"q":{"answer":"yes","evidence":"first turn succeeded"}}"#.to_string()),
        Ok(r#"{"wrong":{"answer":"yes","evidence":"wrong short ID"}}"#.to_string()),
        Ok(r#"{"q":{"answer":"yes","evidence":"fresh retry succeeded"}}"#.to_string()),
    ]);
    let mut caches = CheckRunCaches::new();
    let mut interrogation_session =
        InterrogationSession::new(true, caches.temporary_directory_allocator.clone()).unwrap();

    let first = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();
    let retry = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();

    assert_eq!(first.answer.observed, "yes");
    assert_eq!(retry.answer.observed, "yes");
    assert_eq!(runner.ask_thread_ids, ["thread-1", "thread-1", "thread-2"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
#[test] // xpec: qv
fn first_turn_short_id_mismatch_returns_an_error_without_retry() {
    let root = tests::temp_root("first-turn-short-id-mismatch");
    let config = crate::config_types::CheckConfig {
        version: 1,
        agent: crate::config_types::AgentConfig::implementation_default(),
        expectations: Vec::new(),
    };
    let runtime = CheckRuntime::in_place(&root, &config, true);
    let expectation = temporary_query_expectation(
        &config,
        "Does a first-turn short-ID mismatch avoid a retry?",
    );
    let enforced_scope = crate::hash::full_scope();
    let request = QueryRequest {
        question: &expectation.question,
        enforced_scope: &enforced_scope,
        expectation: QueryExpectationContext {
            expectation: &expectation,
        },
        progress: None,
    };
    let mut runner = tests::FakeQueryRunner::with_turn_results(vec![Ok(
        r#"{"wrong":{"answer":"yes","evidence":"wrong short ID"}}"#.to_string(),
    )]);
    let mut caches = CheckRunCaches::new();
    let mut interrogation_session =
        InterrogationSession::new(true, caches.temporary_directory_allocator.clone()).unwrap();

    let result = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();

    assert!(result.answer.error.is_some());
    assert!(result
        .answer
        .evidence
        .as_deref()
        .is_some_and(|evidence| evidence.contains("short ID")));
    assert_eq!(runner.ask_thread_ids, ["thread-1"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
fn temporary_query_expectation(
    config: &crate::config_types::CheckConfig,
    question: &str,
) -> ResolvedExpectation {
    ResolvedExpectation {
        kind: crate::check::core::ResolvedExpectationKind::TemporaryQuery,
        display_id: "q".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: question.to_string(),
        expected_answer: String::new(),
        question_context: String::new(),
        diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
        target: None,
        agent: config.agent.clone(),
        cooldown: None,
        q_scope: Default::default(),
    }
}
