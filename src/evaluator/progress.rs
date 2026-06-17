use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct EvaluatorProgress {
    state: Arc<Mutex<EvaluatorProgressState>>,
}

#[derive(Default)]
struct EvaluatorProgressState {
    app_server_activity: u64,
    turn_attempt_failure: u64,
    idle_accumulating: bool,
    full_scope_retry: u64,
    q_scope_verification: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EvaluatorProgressSnapshot {
    app_server_activity: u64,
    turn_attempt_failure: u64,
    idle_accumulating: bool,
    full_scope_retry: u64,
    q_scope_verification: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorProgressMarker {
    TurnAttemptFailure,
    Idle,
    FullScopeRetry,
    QScopeVerification,
    AppServerActivity,
}

impl EvaluatorProgress {
    pub(crate) fn new() -> EvaluatorProgress {
        EvaluatorProgress::default()
    }

    pub(crate) fn record_app_server_activity(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
            state.app_server_activity = state.app_server_activity.saturating_add(1);
        }
    }

    pub(crate) fn record_turn_attempt_failure(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
            state.turn_attempt_failure = state.turn_attempt_failure.saturating_add(1);
        }
    }

    pub(crate) fn record_no_app_server_activity_warning(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = true;
        }
    }

    pub(crate) fn record_full_scope_retry_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.full_scope_retry = state.full_scope_retry.saturating_add(1);
        }
    }

    pub(crate) fn record_q_scope_verification_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.q_scope_verification = state.q_scope_verification.saturating_add(1);
        }
    }

    pub(crate) fn snapshot(&self) -> EvaluatorProgressSnapshot {
        self.with_snapshot(|snapshot| snapshot).unwrap_or_default()
    }

    pub(crate) fn with_snapshot<T>(
        &self,
        read: impl FnOnce(EvaluatorProgressSnapshot) -> T,
    ) -> Result<T, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "evaluator progress state poisoned".to_string())?;
        Ok(read(state.snapshot()))
    }
}

impl EvaluatorProgressState {
    fn snapshot(&self) -> EvaluatorProgressSnapshot {
        EvaluatorProgressSnapshot {
            app_server_activity: self.app_server_activity,
            turn_attempt_failure: self.turn_attempt_failure,
            idle_accumulating: self.idle_accumulating,
            full_scope_retry: self.full_scope_retry,
            q_scope_verification: self.q_scope_verification,
        }
    }
}

impl EvaluatorProgressSnapshot {
    pub(crate) fn has_event_since(self, previous: EvaluatorProgressSnapshot) -> bool {
        self.app_server_activity != previous.app_server_activity
            || self.turn_attempt_failure != previous.turn_attempt_failure
            || self.idle_accumulating != previous.idle_accumulating
            || self.full_scope_retry != previous.full_scope_retry
            || self.q_scope_verification != previous.q_scope_verification
    }

    pub(crate) fn marker_since(
        self,
        previous: EvaluatorProgressSnapshot,
    ) -> EvaluatorProgressMarker {
        if self.turn_attempt_failure != previous.turn_attempt_failure {
            return EvaluatorProgressMarker::TurnAttemptFailure;
        }
        if self.idle_accumulating && self.app_server_activity == previous.app_server_activity {
            return EvaluatorProgressMarker::Idle;
        }
        if self.full_scope_retry != previous.full_scope_retry {
            return EvaluatorProgressMarker::FullScopeRetry;
        }
        if self.q_scope_verification != previous.q_scope_verification {
            return EvaluatorProgressMarker::QScopeVerification;
        }
        EvaluatorProgressMarker::AppServerActivity
    }
}

impl EvaluatorProgressMarker {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EvaluatorProgressMarker::TurnAttemptFailure => "×",
            EvaluatorProgressMarker::Idle => "~",
            EvaluatorProgressMarker::FullScopeRetry => "↗",
            EvaluatorProgressMarker::QScopeVerification => "↘",
            EvaluatorProgressMarker::AppServerActivity => ".",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EvaluatorProgress, EvaluatorProgressMarker};

    #[test]
    fn marker_priority_matches_check_timeline_spec() {
        let progress = EvaluatorProgress::new();
        let before = progress.snapshot();

        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::AppServerActivity
        );

        progress.record_no_app_server_activity_warning();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::Idle
        );
        let still_idle = progress.snapshot();
        assert_eq!(
            progress.snapshot().marker_since(still_idle),
            EvaluatorProgressMarker::Idle
        );

        progress.record_app_server_activity();
        assert_eq!(
            progress.snapshot().marker_since(still_idle),
            EvaluatorProgressMarker::AppServerActivity
        );
        let before = progress.snapshot();

        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::AppServerActivity
        );

        progress.record_app_server_activity();
        progress.record_full_scope_retry_started();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::FullScopeRetry
        );

        progress.record_app_server_activity();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::FullScopeRetry
        );

        progress.record_q_scope_verification_started();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::FullScopeRetry
        );

        progress.record_turn_attempt_failure();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::TurnAttemptFailure
        );
    }
}
