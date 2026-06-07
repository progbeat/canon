use crate::check::{
    CheckRecord, CheckRecordOutcome, CheckResult, ParsedAnswer, SelectedExpectation,
};
use crate::config_types::AgentConfig;
use crate::evaluator::types::EvaluatorError;

mod attempt;
mod logging;
mod parse;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use attempt::ask_once;
pub(crate) use logging::{write_thread_lifecycle_event, write_thread_restart_event};
pub(crate) use types::{EvaluatorTurnContext, ParsedTurnResponse, ThreadLifecycleLog};

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
