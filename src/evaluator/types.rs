use crate::config_types::AgentConfig;
use crate::evaluator::turn::EvaluatorFailureKind;
use crate::logs::DiagnosticLogError;
use crate::token_usage_types::EvaluatorTurnUsage;
use std::path::Path;

pub(crate) trait EvaluatorRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        developer_instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError>;
    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError>;

    // Returns usage for the last app-server turn when a turn id was created.
    // Runtime-logged successful turns must provide this. `None` is only valid
    // for failures before an evaluator turn existed.
    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage>;

    fn take_retired_sessions(&mut self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvaluatorError {
    kind: Option<EvaluatorFailureKind>,
    message: String,
}

impl EvaluatorError {
    pub(crate) fn message(message: impl Into<String>) -> EvaluatorError {
        EvaluatorError {
            kind: None,
            message: message.into(),
        }
    }

    pub(crate) fn failure(
        kind: EvaluatorFailureKind,
        message: impl Into<String>,
    ) -> EvaluatorError {
        EvaluatorError {
            kind: Some(kind),
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> Option<EvaluatorFailureKind> {
        self.kind
    }

    pub(crate) fn message_str(&self) -> &str {
        &self.message
    }
}

impl From<String> for EvaluatorError {
    fn from(message: String) -> EvaluatorError {
        EvaluatorError::message(message)
    }
}

impl From<DiagnosticLogError> for EvaluatorError {
    fn from(err: DiagnosticLogError) -> EvaluatorError {
        EvaluatorError::message(err.to_string())
    }
}

impl From<&str> for EvaluatorError {
    fn from(message: &str) -> EvaluatorError {
        EvaluatorError::message(message)
    }
}

impl std::fmt::Display for EvaluatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
