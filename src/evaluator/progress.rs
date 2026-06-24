use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Shared progress handle for one evaluated expectation. Check execution owns the
// handle, installs it on the evaluator runner, and records model fallback plus
// q-scope/full-scope follow-up starts. App-server transport records activity,
// no-progress timeout accumulation, and exhausted no-progress turn timeouts
// through the same handle. Query mode does not create this handle because it
// does not emit per-expectation result-entry timelines.
#[derive(Clone, Default)]
pub(crate) struct EvaluatorProgress {
    state: Arc<Mutex<EvaluatorProgressState>>,
}

#[derive(Default)]
struct EvaluatorProgressState {
    events: Vec<EvaluatorProgressEvent>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvaluatorProgressEventKind {
    TurnTimeout,
    Idle,
    ModelFallback,
    FullScopeRetry,
    QScopeVerification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EvaluatorProgressEvent {
    at: Instant,
    kind: EvaluatorProgressEventKind,
}

impl EvaluatorProgress {
    pub(crate) fn new() -> EvaluatorProgress {
        EvaluatorProgress::default()
    }

    pub(crate) fn record_app_server_activity(&self) {
        // A quiet elapsed window emits "." by default. Higher-priority events
        // are the only state the progress timeline has to remember.
    }

    pub(crate) fn record_turn_timeout(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_turn_timeout_at(Instant::now());
        }
    }

    pub(crate) fn record_no_progress_timeout_accumulating(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_no_progress_timeout_accumulating_at(Instant::now());
        }
    }

    pub(crate) fn record_model_fallback_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_model_fallback_started_at(Instant::now());
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

    pub(crate) fn elapsed_markers_due(
        &self,
        next_marker_at: &mut Instant,
        now: Instant,
        interval: Duration,
    ) -> Result<Vec<EvaluatorProgressMarker>, String> {
        if interval.is_zero() {
            return Ok(Vec::new());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "evaluator progress state poisoned".to_string())?;
        let mut markers = Vec::new();
        let mut latest_closed_marker_at = None;
        while *next_marker_at <= now {
            let marker_at = *next_marker_at;
            let window_start = marker_at - interval;
            markers.push(state.marker_for_window(window_start, marker_at));
            latest_closed_marker_at = Some(marker_at);
            *next_marker_at += interval;
        }
        if let Some(closed_at) = latest_closed_marker_at {
            state.events.retain(|event| event.at > closed_at);
        }
        Ok(markers)
    }
}

impl EvaluatorProgressState {
    fn record_turn_timeout_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::TurnTimeout);
    }

    fn record_no_progress_timeout_accumulating_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::Idle);
    }

    fn record_model_fallback_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::ModelFallback);
    }

    fn record_full_scope_retry_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::FullScopeRetry);
    }

    fn record_q_scope_verification_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::QScopeVerification);
    }

    fn record_marker_event(&mut self, at: Instant, kind: EvaluatorProgressEventKind) {
        self.events.push(EvaluatorProgressEvent { at, kind });
    }

    fn marker_for_window(
        &self,
        window_start: Instant,
        marker_at: Instant,
    ) -> EvaluatorProgressMarker {
        for (kind, marker) in [
            (
                EvaluatorProgressEventKind::TurnTimeout,
                EvaluatorProgressMarker::TurnTimeout,
            ),
            (
                EvaluatorProgressEventKind::Idle,
                EvaluatorProgressMarker::Idle,
            ),
            (
                EvaluatorProgressEventKind::ModelFallback,
                EvaluatorProgressMarker::ModelFallback,
            ),
            (
                EvaluatorProgressEventKind::FullScopeRetry,
                EvaluatorProgressMarker::FullScopeRetry,
            ),
            (
                EvaluatorProgressEventKind::QScopeVerification,
                EvaluatorProgressMarker::QScopeVerification,
            ),
        ] {
            if self
                .events
                .iter()
                .any(|event| event.kind == kind && event.at > window_start && event.at <= marker_at)
            {
                return marker;
            }
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
