use crate::check::core::{
    ParsedAnswer, QueryResult, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::policy::question_scope_suggestion_scope_for_independent_verification;
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{
    ask_with_reused_thread, finalize_query_answer, run_with_model_fallbacks,
    write_query_result_event, write_query_review_required_event, ThreadTurnRequest,
};
use crate::evaluator::{
    create_prompt_template_output_dir, evaluator_turn_prompt, EvaluatorError, EvaluatorRunner,
};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    // Query lifecycle start/finish events are emitted by
    // `check::command::execution::query` so they bracket scope parsing and
    // execution preparation as well as the evaluator turn managed here.
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        None,
        |state, diagnostic_log, model| {
            ask_with_model(
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

fn ask_with_model<R: EvaluatorRunner>(
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
    let attempt = ask_with_full_scope_retry(
        runtime,
        query.question,
        &mut active_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    let mut result = attempt.result;
    let proposed_q_scope =
        // `ScopeTooNarrow` full-scope retry and q-scope verification share the
        // same single follow-up budget in query mode too.
        if should_attempt_q_scope_verification(attempt.follow_up_used, &result.answer) {
            scope_for_verification(runtime, state, &active_scope, &result.answer)?
        } else {
            None
        };
    if let Some(proposed_scope) = proposed_q_scope {
        let narrowed = ask_once(
            runtime,
            query.question,
            &proposed_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
        if answer_is_accepted(&narrowed.answer) {
            result = narrowed;
            result.answer.question_scope_suggestion = None;
        }
    }
    if let Some(reason) = human_review_reason(&result) {
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)?;
        return Err(EvaluatorError::message(format!(
            "query requires human review: {}",
            reason
        )));
    }
    write_query_result_event(query.question, diagnostic_log, &result.answer)?;
    Ok(result)
}

struct QueryAttempt {
    result: QueryResult,
    follow_up_used: bool,
}

fn ask_with_full_scope_retry<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &mut Vec<String>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryAttempt, EvaluatorError> {
    let mut result = ask_once(
        runtime,
        question,
        enforced_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    let mut follow_up_used = false;
    if should_retry_full_scope_after_error(result.answer.error.as_deref(), enforced_scope) {
        // Restricted ScopeTooNarrow is not final for query-mode
        // interrogations either; retry once with full project scope.
        *enforced_scope = full_scope();
        follow_up_used = true;
        result = ask_once(
            runtime,
            question,
            enforced_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
    }
    Ok(QueryAttempt {
        result,
        follow_up_used,
    })
}

fn ask_once<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let template_output_dir =
        create_prompt_template_output_dir().map_err(EvaluatorError::message)?;
    let prompt =
        evaluator_turn_prompt(runtime.root, &template_output_dir, question, "", None, None)?;
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
            expectation_instructions: "",
            prompt: &prompt,
            template_output_dir: &template_output_dir,
            last_pass: None,
        },
    )?;
    finalize_query_answer(runtime, state, enforced_scope, question, response.answer)
}

fn scope_for_verification(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    answer: &ParsedAnswer,
) -> Result<Option<Vec<String>>, EvaluatorError> {
    if answer.error.is_some() {
        return Ok(None);
    }
    question_scope_suggestion_scope_for_independent_verification(
        runtime,
        &runtime.config.agent,
        answer.question_scope_suggestion.as_deref(),
        enforced_scope,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::from)
}

fn answer_is_accepted(narrowed: &ParsedAnswer) -> bool {
    narrowed.error.is_none()
}

fn should_attempt_q_scope_verification(follow_up_used: bool, answer: &ParsedAnswer) -> bool {
    !follow_up_used && answer.error.is_none()
}

fn human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match result.answer.error.as_deref() {
        Some(ERROR_SCOPE_TOO_NARROW) => Some("scope too narrow"),
        Some(ERROR_INVALID_QUESTION) => Some("invalid question"),
        Some(INTERNAL_ERROR_UNPARSABLE) => Some("unparsable evaluator response"),
        None => None,
        Some(_) => Some("unknown evaluator error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_scope_verification_uses_same_follow_up_budget_as_full_scope_retry() {
        let answer = ParsedAnswer::answer(
            "yes".to_string(),
            "evidence".to_string(),
            Some(full_scope()),
        );
        let error = ParsedAnswer::error(ERROR_SCOPE_TOO_NARROW.to_string(), "evidence".to_string());

        assert!(should_attempt_q_scope_verification(false, &answer));
        assert!(!should_attempt_q_scope_verification(true, &answer));
        assert!(!should_attempt_q_scope_verification(false, &error));
    }
}
