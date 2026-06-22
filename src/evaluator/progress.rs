use std::sync::{Arc, Mutex};

// Shared progress handle for one evaluated expectation. Check execution owns the
// handle, installs it on the evaluator runner, and records model fallback plus
// q-scope/full-scope follow-up starts. App-server transport records activity,
// no-progress timeout accumulation, and exhausted no-progress turn timeouts
// through the same handle.
#[derive(Clone, Default)]
pub(crate) struct EvaluatorProgress {
    state: Arc<Mutex<EvaluatorProgressState>>,
}

#[derive(Default)]
struct EvaluatorProgressState {
    app_server_activity: u64,
    turn_timeout: u64,
    idle_accumulating: bool,
    idle_accumulation: u64,
    model_fallback: u64,
    full_scope_retry: u64,
    q_scope_verification: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EvaluatorProgressSnapshot {
    app_server_activity: u64,
    turn_timeout: u64,
    idle_accumulating: bool,
    idle_accumulation: u64,
    model_fallback: u64,
    full_scope_retry: u64,
    q_scope_verification: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorProgressMarker {
    TurnTimeout,
    Idle,
    ModelFallback,
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

    pub(crate) fn record_turn_timeout(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
            state.turn_timeout = state.turn_timeout.saturating_add(1);
        }
    }

    pub(crate) fn record_no_progress_timeout_accumulating(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = true;
            state.idle_accumulation = state.idle_accumulation.saturating_add(1);
        }
    }

    pub(crate) fn record_model_fallback_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
            state.model_fallback = state.model_fallback.saturating_add(1);
        }
    }

    pub(crate) fn record_full_scope_retry_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
            state.full_scope_retry = state.full_scope_retry.saturating_add(1);
        }
    }

    pub(crate) fn record_q_scope_verification_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.idle_accumulating = false;
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
            turn_timeout: self.turn_timeout,
            idle_accumulating: self.idle_accumulating,
            idle_accumulation: self.idle_accumulation,
            model_fallback: self.model_fallback,
            full_scope_retry: self.full_scope_retry,
            q_scope_verification: self.q_scope_verification,
        }
    }
}

impl EvaluatorProgressSnapshot {
    pub(crate) fn marker_since(
        self,
        previous: EvaluatorProgressSnapshot,
    ) -> EvaluatorProgressMarker {
        if self.turn_timeout != previous.turn_timeout {
            return EvaluatorProgressMarker::TurnTimeout;
        }
        if self.idle_accumulation != previous.idle_accumulation
            || (self.idle_accumulating && self.app_server_activity == previous.app_server_activity)
        {
            return EvaluatorProgressMarker::Idle;
        }
        if self.model_fallback != previous.model_fallback {
            return EvaluatorProgressMarker::ModelFallback;
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
            EvaluatorProgressMarker::TurnTimeout => "×",
            EvaluatorProgressMarker::Idle => "~",
            EvaluatorProgressMarker::ModelFallback => "⇄",
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

        progress.record_no_progress_timeout_accumulating();
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

        progress.record_no_progress_timeout_accumulating();
        progress.record_app_server_activity();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::Idle
        );
        let before = progress.snapshot();

        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::AppServerActivity
        );

        progress.record_no_progress_timeout_accumulating();
        let idle_before_fallback = progress.snapshot();
        progress.record_model_fallback_started();
        assert_eq!(
            progress.snapshot().marker_since(idle_before_fallback),
            EvaluatorProgressMarker::ModelFallback
        );
        let before = progress.snapshot();

        progress.record_app_server_activity();
        progress.record_model_fallback_started();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::ModelFallback
        );

        progress.record_full_scope_retry_started();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::ModelFallback
        );

        let before = progress.snapshot();
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

        progress.record_turn_timeout();
        assert_eq!(
            progress.snapshot().marker_since(before),
            EvaluatorProgressMarker::TurnTimeout
        );
    }
}
