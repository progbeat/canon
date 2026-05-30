use crate::check::interrogation::{ask_with_reused_thread, ThreadTurnRequest};
use crate::check::interrogation_policy::q_scope_suggestion_should_get_independent_verification;
use crate::check::interrogation_records::{
    finalize_query_answer, write_query_result_event, write_query_review_required_event,
};
use crate::check::interrogation_state::{CheckRuntime, InterrogationRunState};
use crate::check::model_fallback::run_with_model_fallbacks;
use crate::check::types::{ObservedAnswerState, ParsedAnswer, QueryResult};
use crate::evaluator::prompt::evaluator_turn_prompt;
use crate::evaluator::types::{EvaluatorError, EvaluatorRunner};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;

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
    // q-scope suggestions are trusted only after an independent verification
    // turn returns a schema-valid answer under the suggested scope.
    let mut active_scope = query.enforced_scope.to_vec();
    let mut result = ask_query_once(
        runtime,
        query.question,
        &active_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if query_should_retry_full_scope_after_restricted_response(&result.answer, &active_scope) {
        // Restricted insufficient-evidence is not final for query-mode
        // interrogations either; retry once with full project scope and let
        // that response drive narrowing or human review.
        active_scope = full_scope();
        result = ask_query_once(
            runtime,
            query.question,
            &active_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
    }
    if query_should_verify_narrowing(runtime, state, &active_scope, &result.answer)? {
        let verification_scope = sanitize_scope(
            result
                .answer
                .q_scope_suggestion
                .as_deref()
                .expect("suggestion was validated before verification"),
            &runtime.config.agent,
        )?;
        let narrowed = ask_query_once(
            runtime,
            query.question,
            &verification_scope,
            runner,
            diagnostic_log,
            state,
            model,
        );
        if let Ok(narrowed) = narrowed {
            if query_narrowed_answer_is_accepted(&narrowed.answer) {
                result = narrowed;
            } else {
                result.answer.q_scope_suggestion = None;
            }
        } else {
            result.answer.q_scope_suggestion = None;
        }
    }
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
    let prompt = evaluator_turn_prompt(question);
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

fn query_should_retry_full_scope_after_restricted_response(
    answer: &ParsedAnswer,
    scope: &[String],
) -> bool {
    scope != full_scope()
        && ObservedAnswerState::from_error(answer.error.as_deref())
            == ObservedAnswerState::InsufficientEvidence
}

fn query_should_verify_narrowing(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    answer: &ParsedAnswer,
) -> Result<bool, EvaluatorError> {
    if !matches!(
        ObservedAnswerState::from_error(answer.error.as_deref()),
        ObservedAnswerState::Answer
    ) {
        return Ok(false);
    }
    q_scope_suggestion_should_get_independent_verification(
        runtime,
        &runtime.config.agent,
        answer.q_scope_suggestion.as_deref(),
        enforced_scope,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::from)
}

fn query_narrowed_answer_is_accepted(narrowed: &ParsedAnswer) -> bool {
    matches!(
        ObservedAnswerState::from_error(narrowed.error.as_deref()),
        ObservedAnswerState::Answer
    )
}

fn query_human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match ObservedAnswerState::from_error(result.answer.error.as_deref()) {
        ObservedAnswerState::InsufficientEvidence => Some("insufficient evidence"),
        ObservedAnswerState::InvalidQuestion => Some("invalid question"),
        ObservedAnswerState::Unparsable => Some("unparsable evaluator response"),
        ObservedAnswerState::Unknown => Some("unknown evaluator error"),
        ObservedAnswerState::Answer => None,
    }
}
