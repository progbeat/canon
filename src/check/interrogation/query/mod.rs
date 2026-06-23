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
    // Matched q/a queries use pass/fail records for the acceptance matrix;
    // one-off queries can exercise the verification flow but cannot graduate a
    // narrowed result without pass/fail records.
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
        // per-expectation result line, so there is no public progress timeline
        // for model fallback attempts to update.
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
    let template_output_dir =
        create_prompt_template_output_dir().map_err(EvaluatorError::message)?;
    let diff_from = query.resolved_diff_from(runtime)?;
    let prompt = evaluator_turn_prompt(
        runtime.root,
        &template_output_dir,
        query.turn_question(),
        query.expected_answer(),
        query.diff_from(),
        query.target(),
        diff_from.last_pass,
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
            diff_from_tree_oid: &diff_from.tree_oid,
            prompt: &prompt,
            template_output_dir: &template_output_dir,
            last_pass: diff_from.last_pass,
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
    // The query-mode verification follow-up mirrors the expectation acceptance
    // matrix only when the query is matched to a q/a expectation and therefore
    // has CheckRecord pass/fail results to compare. Expectation sequencing
    // lives in `src/check/run/execute/expectation.rs`, and the shared
    // 25%-smaller gate lives in `src/check/interrogation/policy.rs`.
    if narrowed.answer.error.is_some() {
        return false;
    }
    match (&initial.record, &narrowed.record) {
        (Some(initial), Some(narrowed)) => match (initial.record.result, narrowed.record.result) {
            (CheckResult::Fail, CheckResult::Pass) => false,
            (CheckResult::Pass, CheckResult::Pass)
            | (CheckResult::Pass, CheckResult::Fail)
            | (CheckResult::Fail, CheckResult::Fail) => true,
        },
        (None, None) => false,
        _ => false,
    }
}

fn query_verification_error_is_final(narrowed: &QueryResult) -> bool {
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

    fn expectation_instructions(&self) -> &str {
        self.expectation
            .map(|context| context.expectation.instructions.as_str())
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
    use crate::check::core::{CheckRecord, CheckResult, Cooldown, QueryExpectationRecord};
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
    fn matched_query_rejects_fail_to_pass_q_scope_verification() {
        let expectation = test_expectation();
        let initial_fail = test_query_result(&expectation, "no", CheckResult::Fail, None);
        let narrowed_pass = test_query_result(&expectation, "yes", CheckResult::Pass, None);
        let narrowed_fail = test_query_result(&expectation, "no", CheckResult::Fail, None);
        let narrowed_error = test_query_result(
            &expectation,
            ERROR_SCOPE_TOO_NARROW,
            CheckResult::Fail,
            Some(ERROR_SCOPE_TOO_NARROW),
        );

        assert!(!query_narrowed_scope_is_accepted(
            &initial_fail,
            &narrowed_pass
        ));
        assert!(query_narrowed_scope_is_accepted(
            &initial_fail,
            &narrowed_fail
        ));
        assert!(!query_narrowed_scope_is_accepted(
            &initial_fail,
            &narrowed_error
        ));
    }

    #[test]
    fn one_off_query_never_accepts_q_scope_verification_result() {
        let initial = QueryResult {
            answer: ParsedAnswer::answer("no".to_string(), "evidence".to_string(), None),
            record: None,
        };
        let changed_narrowed = QueryResult {
            answer: ParsedAnswer::answer("yes".to_string(), "evidence".to_string(), None),
            record: None,
        };
        let stable_narrowed = QueryResult {
            answer: ParsedAnswer::answer("no".to_string(), "evidence".to_string(), None),
            record: None,
        };
        let error_narrowed = QueryResult {
            answer: ParsedAnswer::error(ERROR_INVALID_QUESTION.to_string(), "evidence".to_string()),
            record: None,
        };
        let scope_too_narrow = QueryResult {
            answer: ParsedAnswer::error(ERROR_SCOPE_TOO_NARROW.to_string(), "evidence".to_string()),
            record: None,
        };

        assert!(!query_narrowed_scope_is_accepted(
            &initial,
            &changed_narrowed
        ));
        assert!(!query_narrowed_scope_is_accepted(
            &initial,
            &stable_narrowed
        ));
        assert!(!query_narrowed_scope_is_accepted(&initial, &error_narrowed));
        assert!(!query_narrowed_scope_is_accepted(
            &initial,
            &scope_too_narrow
        ));
        assert!(!query_verification_error_is_final(&scope_too_narrow));
        assert!(query_verification_error_is_final(&error_narrowed));
    }

    #[test]
    fn final_query_result_rejects_scope_too_narrow_before_review_handling() {
        let expectation = test_expectation();
        let result = test_query_result(
            &expectation,
            ERROR_SCOPE_TOO_NARROW,
            CheckResult::Fail,
            Some(ERROR_SCOPE_TOO_NARROW),
        );

        let error = assert_final_query_result_has_no_scope_too_narrow(&result)
            .expect_err("final ScopeTooNarrow must be rejected before output");

        assert!(error.contains("forbidden final query scope error"));
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
    }

    fn test_expectation() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "expectation-id".to_string(),
            display_id: "e".to_string(),
            question: "Does matched expectation pass?".to_string(),
            expected_answer: "yes".to_string(),
            instructions: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: true,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }

    fn test_query_result(
        expectation: &SelectedExpectation,
        observed: &str,
        result: CheckResult,
        error: Option<&str>,
    ) -> QueryResult {
        QueryResult {
            answer: if let Some(error) = error {
                ParsedAnswer::error(error.to_string(), "evidence".to_string())
            } else {
                ParsedAnswer::answer(observed.to_string(), "evidence".to_string(), None)
            },
            record: Some(QueryExpectationRecord {
                expectation: expectation.clone(),
                record: CheckRecord {
                    timestamp: "1970-01-01T00:00:00Z".to_string(),
                    number: expectation.number,
                    result,
                    question: Some(expectation.question.clone()),
                    expected_answer: Some(expectation.expected_answer.clone()),
                    observed: observed.to_string(),
                    error: error.map(str::to_string),
                    evidence: "evidence".to_string(),
                    scope: full_scope(),
                    question_scope_suggestion: None,
                    visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    id: expectation.id.clone(),
                    display_id: expectation.display_id.clone(),
                },
            }),
        }
    }
}
