use crate::evaluator::EvaluatorError;

pub(crate) fn is_technical_failure(err: &EvaluatorError) -> bool {
    err.kind().is_some_and(EvaluatorFailureKind::is_technical)
}

pub(crate) fn is_interrupted(err: &EvaluatorError) -> bool {
    err.kind() == Some(EvaluatorFailureKind::Interrupted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorFailureKind {
    UsageLimit,
    RateLimit,
    ModelUnavailable,
    TurnTimeout,
    ContextWindow,
    ShortIdResponse,
    UnknownAppServer,
    Interrupted,
}

impl EvaluatorFailureKind {
    pub(crate) fn is_technical(self) -> bool {
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
        self.is_technical() || matches!(self, EvaluatorFailureKind::ShortIdResponse)
    }
}
