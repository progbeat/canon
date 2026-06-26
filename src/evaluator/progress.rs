use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Shared progress handle for one evaluated expectation. Check execution owns the
// handle, installs it on the evaluator runner, and records the non-initial
// evaluator work kinds that have canon timeline markers: model fallback,
// short-ID response retry, full-scope retry, and q-scope verification. The
// `check::interrogation::session::thread` records fresh-thread retries after
// short-ID response errors through `record_short_id_response_retry_started`.
// The evaluated-expectation path records q-scope verification through
// `record_q_scope_verification_started`; the stdout live timeline writer in
// `check::command::output::record` renders and flushes the due marker.
// The evaluator turn request path records active-turn idle accumulation and
// exhausted no-progress turn timeouts.
// App-server control messages such as initialize and thread/start are session
// setup, not evaluator work kinds in this timeline. When a non-initial
// interrogation needs a fresh session, its caller records the fallback, retry,
// or verification marker before sending thread/start.
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
    ShortIdResponseRetry,
    FullScopeRetry,
    QScopeVerification,
    NoHigherPriorityEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvaluatorProgressEventKind {
    TurnTimeout,
    Idle,
    ModelFallback,
    ShortIdResponseRetry,
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

    pub(crate) fn record_turn_message_activity(&self) {
        // A quiet elapsed window emits "." by default. Higher-priority events
        // are the only state the progress timeline stores.
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

    pub(crate) fn record_short_id_response_retry_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.record_short_id_response_retry_started_at(Instant::now());
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

    pub(crate) fn elapsed_marker_due(
        &self,
        next_marker_at: &mut Instant,
        now: Instant,
        interval: Duration,
    ) -> Result<Option<EvaluatorProgressMarker>, String> {
        if interval.is_zero() || *next_marker_at > now {
            return Ok(None);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "evaluator progress state poisoned".to_string())?;
        // One call classifies one scheduled elapsed-marker tick. The output
        // worker owns the one-minute cadence; this helper owns marker priority.
        let window_start = now - interval;
        let marker = state.marker_for_window(window_start, now);
        state.events.retain(|event| event.at > now);
        *next_marker_at = now + interval;
        Ok(Some(marker))
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

    fn record_short_id_response_retry_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::ShortIdResponseRetry);
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
                EvaluatorProgressEventKind::ShortIdResponseRetry,
                EvaluatorProgressMarker::ShortIdResponseRetry,
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
            if self.events.iter().any(|event| {
                event.kind == kind && event.at >= window_start && event.at <= marker_at
            }) {
                return marker;
            }
        }
        EvaluatorProgressMarker::NoHigherPriorityEvent
    }
}

impl EvaluatorProgressMarker {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EvaluatorProgressMarker::TurnTimeout => "×",
            EvaluatorProgressMarker::Idle => "~",
            EvaluatorProgressMarker::ModelFallback => "⇄",
            EvaluatorProgressMarker::ShortIdResponseRetry => "↻",
            EvaluatorProgressMarker::FullScopeRetry => "↗",
            EvaluatorProgressMarker::QScopeVerification => "↘",
            EvaluatorProgressMarker::NoHigherPriorityEvent => ".",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EvaluatorProgress, EvaluatorProgressMarker};
    use std::time::{Duration, Instant};

    #[test]
    fn elapsed_marker_due_waits_until_scheduled_tick() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        assert_eq!(
            progress
                .elapsed_marker_due(
                    &mut next_marker_at,
                    start + Duration::from_secs(59),
                    interval
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn elapsed_marker_due_classifies_the_current_tick_window() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let event_at = start + Duration::from_secs(30);
        let tick_at = start + interval;
        let mut next_marker_at = start + interval;

        progress
            .state
            .lock()
            .unwrap()
            .record_full_scope_retry_started_at(event_at);

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::FullScopeRetry));
        assert!(next_marker_at > tick_at);
    }

    #[test]
    fn short_id_response_retry_marker_has_canon_priority() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_full_scope_retry_started_at(start + Duration::from_secs(10));
            state.record_short_id_response_retry_started_at(start + Duration::from_secs(20));
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::ShortIdResponseRetry));
        assert_eq!(EvaluatorProgressMarker::ShortIdResponseRetry.as_str(), "↻");
    }

    #[test]
    fn q_scope_verification_marker_uses_canon_symbol() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        progress
            .state
            .lock()
            .unwrap()
            .record_q_scope_verification_started_at(start + Duration::from_secs(30));

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::QScopeVerification));
        assert_eq!(EvaluatorProgressMarker::QScopeVerification.as_str(), "↘");
    }

    #[test]
    fn elapsed_marker_due_includes_window_start_boundary() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        progress
            .state
            .lock()
            .unwrap()
            .record_q_scope_verification_started_at(start);

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::QScopeVerification));
    }
}
