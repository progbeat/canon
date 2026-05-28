use crate::check_interrogation::{ask_with_reused_thread, ThreadTurnRequest};
use crate::check_interrogation_records::{
    finalize_query_answer, write_query_result_event, write_query_review_required_event,
};
use crate::check_interrogation_state::{CheckRuntime, InterrogationRunState};
use crate::check_model_fallback::run_with_model_fallbacks;
use crate::check_types::{ObservedAnswerState, QueryResult};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::logging::DiagnosticLogWriter;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    _expected_answer: Option<&str>,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        None,
        |state, diagnostic_log, model| {
            ask_query_with_model(
                runtime,
                QueryRequest {
                    question,
                    enforced_scope,
                },
                runner,
                diagnostic_log,
                state,
                model,
            )
        },
    )
}

pub(crate) fn ask_query_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    // `canon check -q` uses the same evaluator input shape as normal checks.
    // When the query text maps to one configured expectation, the caller can
    // provide its hidden expected answer so post-response narrowing follows the
    // same "unchanged or still incorrect" rule without adding expected text to
    // the evaluator task input. Pure ad-hoc queries have no expected answer, so
    // changed narrowed answers are not trusted as reusable narrower results.
    let result = ask_query_once(
        runtime,
        query.question,
        query.enforced_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if let Some(reason) = query_human_review_reason(&result) {
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)?;
        return Err(EvaluatorError::message(format!(
            "query requires human review: {}",
            reason
        )));
    }
    write_query_result_event(query.question, diagnostic_log, &result.answer)?;
    Ok(result)
}

fn ask_query_once<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let prompt = question.to_string();
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent: &runtime.config.agent,
            enforced_scope,
            model,
            thinking: &runtime.config.agent.thinking,
            expectation_id: None,
            prompt: &prompt,
        },
    )?;
    finalize_query_answer(runtime, state, enforced_scope, question, response.answer)
}

fn query_human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match ObservedAnswerState::from_observed(&result.answer.answer) {
        ObservedAnswerState::InsufficientEvidence => Some("insufficient evidence"),
        ObservedAnswerState::InvalidQuestion => Some("invalid question"),
        ObservedAnswerState::Unparsable => Some("unparsable evaluator response"),
        ObservedAnswerState::Unknown => Some("unknown observed answer state"),
        ObservedAnswerState::Answer => None,
    }
}
