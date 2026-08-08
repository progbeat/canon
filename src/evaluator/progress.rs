mod completion;
mod elapsed;
mod state;

use self::state::EvaluatorProgressState;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const PROGRESS_TIMELINE_MARKER_INTERVAL: Duration = Duration::from_secs(60);

// Shared progress handle for one evaluated expectation. Check execution owns the
// handle, installs it on the evaluator runner, and records the non-initial
// evaluator work kinds that have canon timeline markers: model fallback,
// fresh-thread retry after a short-ID response error, full-scope retry, and
// q-scope verification. `src/check/interrogation/session/model_fallback.rs`
// records `⇄`; `src/check/interrogation/session/thread/lifecycle/mod.rs`
// records `↻` through
// `record_fresh_thread_retry_after_short_id_response_error_started`;
// `src/check/interrogation/turn_kind.rs` records turn-start `↗` and
// `↘`; `src/check/engine/execute/expectation/policy.rs` records result-side
// `↖` and `⤡`; and `src/app/process/runner/transport/turn.rs` records
// active-turn idle accumulation `~` and exhausted no-progress turn timeouts
// `×`.
// The stdout live timeline writer in `check::command::output::record` renders
// and flushes the due marker.
// App-server control messages such as initialize and thread/start are thread
// setup, not evaluator work kinds in this timeline. When a non-initial
// interrogation needs a fresh thread, its caller records the fallback, retry,
// or verification marker before sending thread/start.
#[derive(Clone, Default)]
pub(crate) struct EvaluatorProgress {
    state: Arc<Mutex<EvaluatorProgressState>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorProgressMarker {
    TurnTimeout,
    Idle,
    ModelFallback,
    // Canon `↻`: a fresh-thread retry after a short-ID response error started.
    FreshThreadRetryAfterShortIdResponseError,
    FullScopeRetry,
    QScopeVerificationStartedAndReturnedScopeTooNarrow,
    QScopeVerificationReturnedScopeTooNarrow,
    QScopeVerification,
    NoHigherPriorityEvent,
}

impl EvaluatorProgress {
    pub(crate) fn new() -> EvaluatorProgress {
        EvaluatorProgress::default()
    }

    pub(crate) fn record_turn_attempt_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_turn_attempt_started_at(Instant::now());
        }
    }

    pub(crate) fn record_turn_message_activity(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_turn_message_activity_at(Instant::now());
        }
    }

    pub(crate) fn record_turn_attempt_finished(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_turn_attempt_finished_at(Instant::now());
        }
    }

    pub(crate) fn record_turn_timeout(&self) {
        if let Ok(mut state) = self.state.lock() {
            let now = Instant::now();
            state.assert_turn_timeout_guarantees_preceding_idle_marker(
                now,
                PROGRESS_TIMELINE_MARKER_INTERVAL,
            );
            state.record_turn_timeout_at(now);
        }
    }

    pub(crate) fn record_model_fallback_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_model_fallback_started_at(Instant::now());
        }
    }

    pub(crate) fn record_fresh_thread_retry_after_short_id_response_error_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state
                .record_fresh_thread_retry_after_short_id_response_error_started_at(Instant::now());
        }
    }

    pub(crate) fn record_full_scope_retry_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_full_scope_retry_started_at(Instant::now());
        }
    }

    pub(crate) fn record_q_scope_verification_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_q_scope_verification_started_at(Instant::now());
        }
    }

    pub(crate) fn record_q_scope_verification_returned_scope_too_narrow(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_q_scope_verification_returned_scope_too_narrow_at(Instant::now());
        }
    }
}

impl EvaluatorProgressMarker {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EvaluatorProgressMarker::TurnTimeout => "×",
            EvaluatorProgressMarker::Idle => "~",
            EvaluatorProgressMarker::ModelFallback => "⇄",
            EvaluatorProgressMarker::FreshThreadRetryAfterShortIdResponseError => "↻",
            EvaluatorProgressMarker::FullScopeRetry => "↗",
            EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow => "⤡",
            EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow => "↖",
            EvaluatorProgressMarker::QScopeVerification => "↘",
            EvaluatorProgressMarker::NoHigherPriorityEvent => ".",
        }
    }
}
