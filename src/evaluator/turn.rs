use crate::check::{
    CheckRecord, CheckRecordOutcome, CheckResult, ParsedAnswer, SelectedExpectation,
};
use crate::config_types::AgentConfig;
use crate::evaluator::response_cache::EvaluatorResponseParseCache;
use crate::evaluator::types::{EvaluatorError, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use crate::token_usage_types::TokenUsage;
use std::path::Path;

mod logging;
mod parse;
#[cfg(test)]
mod tests;

use logging::ask_and_log;
pub(crate) use logging::{write_thread_lifecycle_event, write_thread_restart_event};
use parse::{
    insufficient_evidence_response_answer, parse_visible_evaluator_response,
    unparsable_response_answer, EvaluatorResponseParseError, RESPONSE_REPAIR_PROMPT,
};

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    if agent.models.is_empty() {
        return vec![None];
    }
    agent.models.iter().cloned().map(Some).collect()
}

pub(crate) fn effective_thinking<'a>(
    _agent: &'a AgentConfig,
    expectation: &'a SelectedExpectation,
) -> &'a str {
    &expectation.agent.thinking
}

pub(crate) fn model_label(model: Option<&str>) -> &str {
    model.unwrap_or("<default>")
}

pub(crate) fn is_model_technical_failure(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::is_model_technical)
}

pub(crate) fn is_context_window_failure(err: &EvaluatorError) -> bool {
    err.kind() == Some(EvaluatorFailureKind::ContextWindow)
}

pub(crate) fn session_failure_invalidates_thread(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::invalidates_thread)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorFailureKind {
    UsageLimit,
    RateLimit,
    ModelUnavailable,
    TurnTimeout,
    ContextWindow,
    UnknownAppServer,
}

impl EvaluatorFailureKind {
    pub(crate) fn is_model_technical(self) -> bool {
        matches!(
            self,
            EvaluatorFailureKind::UsageLimit
                | EvaluatorFailureKind::RateLimit
                | EvaluatorFailureKind::ModelUnavailable
                | EvaluatorFailureKind::TurnTimeout
                | EvaluatorFailureKind::ContextWindow
                | EvaluatorFailureKind::UnknownAppServer
        )
    }

    pub(crate) fn invalidates_thread(self) -> bool {
        self.is_model_technical()
    }
}

pub(crate) fn record_from_response(
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    visible_tree_oid: String,
) -> Result<CheckRecord, String> {
    let result = if response.error.is_some() {
        CheckResult::Fail
    } else {
        CheckResult::from_expected_answer(&expectation.a, &response.answer)
    };
    let error = response.error.clone();
    let suggested_q_scope = response.q_scope_suggestion.clone();
    CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            result,
            observed: response.answer,
            error,
            evidence: response.evidence,
            scope: enforced_scope,
            suggested_q_scope,
            visible_tree_oid,
        },
    )
}

// One evaluator turn: model labels, response parsing, request and response
// logging, per-turn token usage, and record finalization.
pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) struct ThreadLifecycleLog {
    pub(crate) event: &'static str,
    pub(crate) session_id: String,
    pub(crate) developer_instructions: String,
}

pub(crate) struct ParsedTurnResponse {
    pub(crate) answer: ParsedAnswer,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
}

pub(crate) struct RawTurnResponse {
    pub(crate) text: String,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
}

pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    visible_scope: &[String],
    session_root: Option<&Path>,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let response = ask_and_log(
        runner,
        turn,
        prompt,
        diagnostic_log,
        expectation_id,
        1,
        "initial",
    )?;
    let mut usage = response.usage;
    let mut context_compacted = response.context_compacted;
    let parsed = match parse_visible_evaluator_response(
        parser_cache,
        &response.text,
        agent,
        visible_scope,
        session_root,
    ) {
        Ok(answer) => answer,
        Err(_) => {
            let repair = ask_and_log(
                runner,
                turn,
                RESPONSE_REPAIR_PROMPT,
                diagnostic_log,
                expectation_id,
                2,
                "repair",
            )?;
            usage = combined_turn_usage(usage, repair.usage);
            context_compacted |= repair.context_compacted;
            match parse_visible_evaluator_response(
                parser_cache,
                &repair.text,
                agent,
                visible_scope,
                session_root,
            ) {
                Ok(answer) => answer,
                Err(EvaluatorResponseParseError::OutOfScopeEvidence) => {
                    insufficient_evidence_response_answer()
                }
                Err(EvaluatorResponseParseError::InvalidResponse(err)) => {
                    unparsable_response_answer(&err, &repair.text)
                }
            }
        }
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        usage,
        context_compacted,
    })
}

fn combined_turn_usage(
    first: Option<TokenUsage>,
    second: Option<TokenUsage>,
) -> Option<TokenUsage> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.add(second)),
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (None, None) => None,
    }
}
