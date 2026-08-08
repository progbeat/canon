use crate::evaluator::{EvaluatorError, EvaluatorFailureKind};
use std::time::{Duration, Instant};

const APP_SERVER_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) enum IdleTimeoutKind {
    Response,
    Progress,
}

pub(super) fn idle_timeout_elapsed(last_activity: Instant, now: Instant) -> bool {
    now.duration_since(last_activity) >= APP_SERVER_IDLE_TIMEOUT
}

pub(super) fn idle_timeout_error(method: &str, kind: IdleTimeoutKind) -> EvaluatorError {
    let missing_activity = match kind {
        IdleTimeoutKind::Response => "response",
        IdleTimeoutKind::Progress => "progress",
    };
    EvaluatorError::failure(
        EvaluatorFailureKind::TurnTimeout,
        format!(
            "app-server {} timed out after {} seconds without {}",
            method,
            APP_SERVER_IDLE_TIMEOUT.as_secs(),
            missing_activity
        ),
    )
}
