use crate::check::core::{
    CheckResult, ParsedAnswer, QueryResult, SelectedExpectation, ERROR_INVALID_QUESTION,
    ERROR_SCOPE_TOO_NARROW, INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::policy::question_scope_suggestion_scope_for_independent_verification;
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{
    ask_with_reused_thread, finalize_query_answer, resolve_diff_from, run_with_model_fallbacks,
    write_query_result_event, write_query_review_required_event, ResolvedDiffFrom,
    ThreadTurnRequest,
};
use crate::config_types::{AgentConfig, DEFAULT_DIFF_FROM};
use crate::evaluator::{
    effective_thinking, evaluator_turn_prompt, EvaluatorError, EvaluatorRunner,
    EvaluatorTurnPromptContext, PromptTemplateArtifactDir,
};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::LastResult;
use std::sync::Arc;

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
    let attempt = match ask_with_full_scope_retry(
        runtime,
        query,
        &mut active_scope,
        runner,
        diagnostic_log,
        state,
    ) {
        Ok(attempt) => attempt,
        Err(error) => QueryAttempt {
            result: query_result_from_interrogation_error(
                runtime,
                query,
                state,
                &active_scope,
                error,
            )?,
            follow_up_used: false,
        },
    };
    // This is query mode's use of the same Interrogation Policy q-scope
    // verification follow-up, not a separate `qScopeSuggestion` decision.
    // Matched q/a queries use expectation pass/fail records; one-off queries
    // derive verification-only pass/fail from whether the answer still matches
    // the initial answer.
    let q_scope_verification_scope =
        q_scope_verification_scope_for_query_answer(runtime, query, state, &active_scope, &attempt)
            .map_err(|err| err.to_string())?;
    let mut result = attempt.result;
    if let Some(proposed_scope) = q_scope_verification_scope {
        let narrowed = match ask_once(
            runtime,
            query,
            &proposed_scope,
            runner,
            diagnostic_log,
            state,
        ) {
            Ok(narrowed) => narrowed,
            Err(error) => {
                result = query_result_from_interrogation_error(
                    runtime,
                    query,
                    state,
                    &proposed_scope,
                    error,
                )?;
                return finish_query_result(query, diagnostic_log, result);
            }
        };
        if query_verification_error_is_final(&narrowed) {
            result = narrowed;
        } else if query_narrowed_scope_is_accepted(&result, &narrowed) {
            result = narrowed;
            result.answer.question_scope_suggestion = None;
        }
    }
    finish_query_result(query, diagnostic_log, result)
}

fn finish_query_result(
    query: QueryRequest<'_>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    result: QueryResult,
) -> Result<QueryResult, String> {
    assert_final_query_result_has_no_scope_too_narrow(&result)?;
    if let Some(reason) = query_human_review_reason(&result) {
        // The command layer may persist a matched expectation record before it
        // turns this result into a human-review command error.
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)
            .map_err(|err| err.to_string())?;
        return Ok(result);
    }
    // Successful query mode emits query.result directly from the finalized
    // parsed answer.
    write_query_result_event(query.question, diagnostic_log, &result.answer)
        .map_err(|err| err.to_string())?;
    Ok(result)
}

fn assert_final_query_result_has_no_scope_too_narrow(result: &QueryResult) -> Result<(), String> {
    // A final ScopeTooNarrow would be a policy bug: initial restricted
    // query-mode interrogations retry at full scope, full-scope schemas reject
    // ScopeTooNarrow, and q-scope verification errors cannot replace the
    // initial result. Do not rewrite the evaluator-provided error value here;
    // stop before query result output or human-review handling can expose it.
    if result.answer.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        return Err("internal error: forbidden final query scope error".to_string());
    }
    Ok(())
}

fn query_result_from_interrogation_error(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    error: String,
) -> Result<QueryResult, String> {
    finalize_query_answer(
        runtime,
        state,
        query.agent(&runtime.config.agent),
        query.expectation.map(|context| context.expectation),
        enforced_scope,
        query.question,
        ParsedAnswer::error(INTERNAL_ERROR_UNPARSABLE.to_string(), error),
    )
    .map_err(|err| err.to_string())
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
    if !runtime.is_in_place()
        && should_retry_full_scope_after_error(result.answer.error.as_deref(), enforced_scope)
    {
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
        // `canon check -q` returns a query answer instead of emitting a
        // `<short ID><progress timeline>` result entry. Passing `None` here
        // opts out of result-timeline reporting for the whole query command
        // form; it is not a check-run request kind without a marker.
        None,
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
    let diff_from = query.resolved_diff_from(runtime)?;
    let template_artifact_dir =
        PromptTemplateArtifactDir::Lazy(Arc::clone(&state.prompt_template_output_dir_cache));
    let mut template_artifact_paths = Vec::new();
    let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
        root: runtime.root,
        template_artifact_dir: template_artifact_dir.clone(),
        template_artifact_paths: &mut template_artifact_paths,
        short_id: query.short_id(),
        question: query.turn_question(),
        expected_answer: query.expected_answer(),
        in_place: runtime.is_in_place(),
        diff_from: query.diff_from(),
        target: query.target(),
        last_pass: diff_from.last_pass,
    })?;
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
            short_id: query.short_id(),
            question_context: query.question_context(),
            diff_from_tree_oid: &diff_from.tree_oid,
            prompt: &prompt,
            template_artifact_dir,
            template_artifact_paths: &template_artifact_paths,
            last_pass: diff_from.last_pass,
            progress: None,
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
    // `ScopeTooNarrow` full-scope retry and the q-scope verification follow-up
    // share the same single follow-up budget in query mode too.
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

fn query_narrowed_scope_is_accepted(initial: &QueryResult, narrowed: &QueryResult) -> bool {
    let Some(initial_result) =
        query_result_for_q_scope_verification(&initial.answer.answer, initial)
    else {
        return false;
    };
    let Some(narrowed_result) =
        query_result_for_q_scope_verification(&initial.answer.answer, narrowed)
    else {
        return false;
    };
    match (initial_result, narrowed_result) {
        (CheckResult::Fail, CheckResult::Pass) => false,
        (CheckResult::Pass, CheckResult::Pass)
        | (CheckResult::Pass, CheckResult::Fail)
        | (CheckResult::Fail, CheckResult::Fail) => true,
    }
}

fn query_result_for_q_scope_verification(
    initial_answer: &str,
    result: &QueryResult,
) -> Option<CheckResult> {
    if result.answer.error.is_some() {
        return None;
    }
    if let Some(record) = &result.record {
        return Some(record.record.result);
    }
    if result.answer.answer == initial_answer {
        Some(CheckResult::Pass)
    } else {
        Some(CheckResult::Fail)
    }
}

fn query_verification_error_is_final(narrowed: &QueryResult) -> bool {
    // Verification ScopeTooNarrow rejects the proposed narrowed q-scope; it
    // does not replace the initial query answer as the final response.
    narrowed.answer.error.is_some()
        && narrowed.answer.error.as_deref() != Some(ERROR_SCOPE_TOO_NARROW)
}

fn q_scope_verification_follow_up_is_available(
    follow_up_used: bool,
    answer: &ParsedAnswer,
) -> bool {
    !follow_up_used && answer.error.is_none()
}

pub(crate) fn query_human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match result.answer.error.as_deref() {
        Some(ERROR_SCOPE_TOO_NARROW) => {
            unreachable!("final query result cannot expose scope-too-narrow")
        }
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

    fn short_id(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.display_id.as_str())
            .unwrap_or("q")
    }

    fn question_context(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.question_context.as_str())
            .unwrap_or("")
    }

    fn resolved_diff_from(
        &self,
        runtime: &CheckRuntime<'_>,
    ) -> Result<ResolvedDiffFrom<'a>, EvaluatorError> {
        match self.expectation {
            Some(context) => resolve_diff_from(runtime, context.expectation, context.last_pass),
            None => Ok(ResolvedDiffFrom {
                tree_oid: runtime.against_tree_oid().to_string(),
                last_pass: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::Cooldown;
    use crate::config_types::ExpectationTarget;

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
            question_context: "Use this expectation context.".to_string(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            diff_from_configured: false,
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
        assert_eq!(request.question_context(), "Use this expectation context.");
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
        assert_eq!(request.question_context(), "");
    }
}
