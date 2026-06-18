use crate::check::core::{
    ParsedAnswer, QueryResult, SelectedExpectation, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::policy::question_scope_suggestion_scope_for_independent_verification;
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{
    ask_with_reused_thread, finalize_query_answer, resolve_diff_from_tree_oid,
    run_with_model_fallbacks, write_query_result_event, write_query_review_required_event,
    ThreadTurnRequest,
};
use crate::config_types::{AgentConfig, DEFAULT_DIFF_FROM};
use crate::evaluator::{
    create_prompt_template_output_dir, effective_thinking, evaluator_turn_prompt, EvaluatorError,
    EvaluatorRunner,
};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::LastResult;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) expectation: Option<QueryExpectationContext<'a>>,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryExpectationContext<'a> {
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    // Query lifecycle start/finish events are emitted by
    // `check::command::execution::query` so they bracket scope parsing and
    // execution preparation as well as the evaluator turn managed here.
    let mut diagnostic_log = diagnostic_log;
    ask_query(runtime, query, runner, &mut diagnostic_log, state)
}

fn ask_query<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    // `canon check -q` uses the same evaluator input shape as normal checks.
    // When the question exactly matches one q/a-only expectation, query mode
    // reuses that expectation's prompt context so `-q <q>` and `<ID>` begin
    // with the same evaluator input under the same scope.
    // q-scope suggestions are trusted only after an independent verification
    // turn returns a schema-valid answer under the suggested scope.
    let mut active_scope = query.enforced_scope.to_vec();
    let attempt = ask_with_full_scope_retry(
        runtime,
        query,
        &mut active_scope,
        runner,
        diagnostic_log,
        state,
    )?;
    // Query mode uses the same Interrogation Policy q-scope verification
    // follow-up as expectation mode. All query-mode dependence on
    // `qScopeSuggestion` is contained in this verification planning helper.
    let q_scope_verification_scope =
        q_scope_verification_scope_for_query_answer(runtime, query, state, &active_scope, &attempt)
            .map_err(|err| err.to_string())?;
    let mut result = attempt.result;
    if let Some(proposed_scope) = q_scope_verification_scope {
        let narrowed = ask_once(
            runtime,
            query,
            &proposed_scope,
            runner,
            diagnostic_log,
            state,
        )?;
        if answer_is_accepted(&narrowed.answer) {
            result = narrowed;
            result.answer.question_scope_suggestion = None;
        }
    }
    if let Some(reason) = human_review_reason(&result) {
        // Query mode has no CheckRecord, so it emits query.review_required
        // directly from the finalized parsed answer.
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)
            .map_err(|err| err.to_string())?;
        return Err(format!("query requires human review: {}", reason));
    }
    // Successful query mode emits query.result directly from the finalized
    // parsed answer.
    write_query_result_event(query.question, diagnostic_log, &result.answer)
        .map_err(|err| err.to_string())?;
    Ok(result)
}

struct QueryAttempt {
    result: QueryResult,
    follow_up_used: bool,
}

fn ask_with_full_scope_retry<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    enforced_scope: &mut Vec<String>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryAttempt, String> {
    let mut result = ask_once(
        runtime,
        query,
        enforced_scope,
        runner,
        diagnostic_log,
        state,
    )?;
    let mut follow_up_used = false;
    if should_retry_full_scope_after_error(result.answer.error.as_deref(), enforced_scope) {
        // Restricted ScopeTooNarrow is not final for query-mode
        // interrogations either; retry once with full project scope.
        *enforced_scope = full_scope();
        follow_up_used = true;
        result = ask_once(
            runtime,
            query,
            enforced_scope,
            runner,
            diagnostic_log,
            state,
        )?;
    }
    Ok(QueryAttempt {
        result,
        follow_up_used,
    })
}

fn ask_once<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    let agent = query.agent(&runtime.config.agent);
    run_with_model_fallbacks(
        agent,
        state,
        diagnostic_log,
        query.expectation_id(),
        |state, diagnostic_log, model| {
            ask_once_with_model(
                runtime,
                query,
                enforced_scope,
                runner,
                diagnostic_log,
                state,
                model,
            )
        },
    )
}

fn ask_once_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let template_output_dir =
        create_prompt_template_output_dir().map_err(EvaluatorError::message)?;
    let diff_from_tree_oid = query.diff_from_tree_oid(runtime)?;
    let prompt = evaluator_turn_prompt(
        runtime.root,
        &template_output_dir,
        query.turn_question(),
        query.expected_answer(),
        query.diff_from(),
        query.target(),
        query.last_pass(),
    )?;
    let agent = query.agent(&runtime.config.agent);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent,
            enforced_scope,
            model,
            thinking: query.thinking(&runtime.config.agent),
            expectation_id: query.expectation_id(),
            expectation_instructions: query.expectation_instructions(),
            diff_from_tree_oid: &diff_from_tree_oid,
            prompt: &prompt,
            template_output_dir: &template_output_dir,
            last_pass: query.last_pass(),
        },
    )?;
    finalize_query_answer(
        runtime,
        state,
        agent,
        query.expectation.map(|context| context.expectation),
        enforced_scope,
        query.question,
        response.answer,
    )
}

fn q_scope_verification_scope_for_query_answer(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    attempt: &QueryAttempt,
) -> Result<Option<Vec<String>>, EvaluatorError> {
    // `ScopeTooNarrow` full-scope retry and q-scope verification share the
    // same single follow-up budget in query mode too.
    if !q_scope_verification_follow_up_is_available(attempt.follow_up_used, &attempt.result.answer)
    {
        return Ok(None);
    }
    question_scope_suggestion_scope_for_independent_verification(
        runtime,
        query.agent(&runtime.config.agent),
        attempt.result.answer.question_scope_suggestion.as_deref(),
        enforced_scope,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::from)
}

fn answer_is_accepted(narrowed: &ParsedAnswer) -> bool {
    narrowed.error.is_none()
}

fn q_scope_verification_follow_up_is_available(
    follow_up_used: bool,
    answer: &ParsedAnswer,
) -> bool {
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

impl<'a> QueryRequest<'a> {
    fn agent<'b>(&'b self, default_agent: &'b AgentConfig) -> &'b AgentConfig {
        self.expectation
            .map(|context| &context.expectation.agent)
            .unwrap_or(default_agent)
    }

    fn thinking<'b>(&'b self, default_agent: &'b AgentConfig) -> &'b str {
        self.expectation
            .map(|context| effective_thinking(&context.expectation.agent, context.expectation))
            .unwrap_or(&default_agent.thinking)
    }

    fn turn_question(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.question.as_str())
            .unwrap_or(self.question)
    }

    fn expected_answer(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.expected_answer.as_str())
            .unwrap_or("")
    }

    fn target(&self) -> Option<&str> {
        self.expectation
            .and_then(|context| context.expectation.target.as_ref())
            .map(|target| target.as_str())
    }

    fn diff_from(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.diff_from.as_str())
            .unwrap_or(DEFAULT_DIFF_FROM)
    }

    fn expectation_id(&self) -> Option<&str> {
        self.expectation
            .map(|context| context.expectation.id.as_str())
    }

    fn expectation_instructions(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.instructions.as_str())
            .unwrap_or("")
    }

    fn last_pass(&self) -> Option<&LastResult> {
        self.expectation.and_then(|context| context.last_pass)
    }

    fn diff_from_tree_oid(&self, runtime: &CheckRuntime<'_>) -> Result<String, EvaluatorError> {
        match self.expectation {
            Some(context) => {
                resolve_diff_from_tree_oid(runtime, context.expectation, context.last_pass)
            }
            None => Ok(runtime.tree_context.against_tree_oid.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::Cooldown;
    use crate::config_types::ExpectationTarget;

    #[test]
    fn q_scope_verification_uses_same_follow_up_budget_as_full_scope_retry() {
        let answer = ParsedAnswer::answer(
            "yes".to_string(),
            "evidence".to_string(),
            Some(full_scope()),
        );
        let error = ParsedAnswer::error(ERROR_SCOPE_TOO_NARROW.to_string(), "evidence".to_string());

        assert!(q_scope_verification_follow_up_is_available(false, &answer));
        assert!(!q_scope_verification_follow_up_is_available(true, &answer));
        assert!(!q_scope_verification_follow_up_is_available(false, &error));
    }

    #[test]
    fn matched_query_request_uses_expectation_turn_context() {
        let default_agent = AgentConfig::implementation_default();
        let mut expectation_agent = AgentConfig::implementation_default();
        expectation_agent.thinking = "high".to_string();
        let expectation = SelectedExpectation {
            number: 1,
            id: "expectation-id".to_string(),
            display_id: "e".to_string(),
            question: "Does matched expectation pass?".to_string(),
            expected_answer: "yes".to_string(),
            instructions: "Use this expectation context.".to_string(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: Some(ExpectationTarget::Diff),
            question_answer_only: true,
            agent: expectation_agent,
            cooldown: Some(Cooldown {
                pass_seconds: None,
                fail_seconds: None,
            }),
        };
        let request = QueryRequest {
            question: "Does matched expectation pass?",
            enforced_scope: &[],
            expectation: Some(QueryExpectationContext {
                expectation: &expectation,
                last_pass: None,
            }),
        };

        assert_eq!(request.agent(&default_agent).thinking, "high");
        assert_eq!(request.thinking(&default_agent), "high");
        assert_eq!(request.turn_question(), expectation.question);
        assert_eq!(request.expected_answer(), expectation.expected_answer);
        assert_eq!(request.target(), Some("diff"));
        assert_eq!(request.expectation_id(), Some("expectation-id"));
        assert_eq!(
            request.expectation_instructions(),
            "Use this expectation context."
        );
    }

    #[test]
    fn unmatched_query_request_keeps_one_off_context() {
        let default_agent = AgentConfig::implementation_default();
        let request = QueryRequest {
            question: "Does a one-off question pass?",
            enforced_scope: &[],
            expectation: None,
        };

        assert_eq!(
            request.agent(&default_agent).thinking,
            default_agent.thinking
        );
        assert_eq!(request.thinking(&default_agent), default_agent.thinking);
        assert_eq!(request.turn_question(), "Does a one-off question pass?");
        assert_eq!(request.expected_answer(), "");
        assert_eq!(request.target(), None);
        assert_eq!(request.expectation_id(), None);
        assert_eq!(request.expectation_instructions(), "");
        assert!(request.last_pass().is_none());
    }
}
