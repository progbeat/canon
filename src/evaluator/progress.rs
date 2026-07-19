use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Shared progress handle for one evaluated expectation. Check execution owns the
// handle, installs it on the evaluator runner, and records the non-initial
// evaluator work kinds that have canon timeline markers: model fallback,
// fresh-thread retry after a short-ID response error, full-scope retry, and
// q-scope verification. `src/check/interrogation/session/model_fallback.rs`
// records `⇄`; `src/check/interrogation/session/thread.rs` records `↻` through
// `record_fresh_thread_retry_after_short_id_response_error_started`;
// `src/check/interrogation/request_kind.rs` records request-start `↗` and
// `↘`; `src/check/run/execute/expectation.rs` records result-side `↖` and
// `⤡`; and `src/app/process/transport.rs` records active-turn idle
// accumulation `~` and exhausted no-progress turn timeouts `×`.
// The stdout live timeline writer in `check::command::output::record` renders
// and flushes the due marker.
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
    active_no_progress_since: Option<Instant>,
    completed_no_progress_intervals: Vec<NoProgressInterval>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvaluatorProgressEventKind {
    TurnTimeout,
    ModelFallback,
    FreshThreadRetryAfterShortIdResponseError,
    FullScopeRetry,
    QScopeVerification,
    QScopeVerificationReturnedScopeTooNarrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EvaluatorProgressEvent {
    at: Instant,
    kind: EvaluatorProgressEventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoProgressInterval {
    started_at: Instant,
    ended_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressWindowKind {
    CompletedFullMinute,
    FinalMinute,
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
            state.record_turn_timeout_at(Instant::now());
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
        // One call classifies one scheduled elapsed-marker tick. If the output
        // worker wakes late, advancing by the scheduled tick lets zero-duration
        // waits emit the skipped minute markers in order.
        let marker_at = *next_marker_at;
        let window_start = marker_at - interval;
        let marker = state.marker_for_window(
            window_start,
            marker_at,
            ProgressWindowKind::CompletedFullMinute,
        );
        state.prune_events(marker_at, ProgressWindowKind::CompletedFullMinute);
        state.prune_no_progress_intervals(marker_at);
        *next_marker_at += interval;
        Ok(Some(marker))
    }

    pub(crate) fn completion_markers_due(
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
        while *next_marker_at <= now {
            let marker_at = *next_marker_at;
            let window_start = marker_at - interval;
            markers.push(state.marker_for_window(
                window_start,
                marker_at,
                ProgressWindowKind::CompletedFullMinute,
            ));
            state.prune_events(marker_at, ProgressWindowKind::CompletedFullMinute);
            state.prune_no_progress_intervals(marker_at);
            *next_marker_at += interval;
        }
        let final_window_start = *next_marker_at - interval;
        // [03,E] The loop emitted every completed full minute as a half-open
        // elapsed interval. When `now` is exactly a minute boundary,
        // `final_window_start == now`: the required final marker represents the
        // zero-duration minute containing the completion event at that instant.
        // It cannot claim that a timeout is still accumulating, so a terminal
        // timeout at that instant renders `×` here and the suffix remains `~×`.
        markers.push(state.marker_for_window(
            final_window_start,
            now,
            ProgressWindowKind::FinalMinute,
        ));
        state.prune_events(now, ProgressWindowKind::FinalMinute);
        state.prune_no_progress_intervals(now);
        *next_marker_at = now + interval;
        Ok(markers)
    }
}

impl EvaluatorProgressState {
    fn record_turn_attempt_started_at(&mut self, at: Instant) {
        self.finish_active_no_progress_interval_at(at);
        self.active_no_progress_since = Some(at);
    }

    fn record_turn_message_activity_at(&mut self, at: Instant) {
        if self.active_no_progress_since.is_none() {
            return;
        }
        self.finish_active_no_progress_interval_at(at);
        self.active_no_progress_since = Some(at);
    }

    fn record_turn_attempt_finished_at(&mut self, at: Instant) {
        self.finish_active_no_progress_interval_at(at);
    }

    fn record_turn_timeout_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::TurnTimeout);
    }

    fn finish_active_no_progress_interval_at(&mut self, at: Instant) {
        let Some(started_at) = self.active_no_progress_since.take() else {
            return;
        };
        self.completed_no_progress_intervals
            .push(NoProgressInterval {
                started_at,
                ended_at: at,
            });
    }

    fn record_model_fallback_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::ModelFallback);
    }

    fn record_fresh_thread_retry_after_short_id_response_error_started_at(&mut self, at: Instant) {
        self.record_marker_event(
            at,
            EvaluatorProgressEventKind::FreshThreadRetryAfterShortIdResponseError,
        );
    }

    fn record_full_scope_retry_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::FullScopeRetry);
    }

    fn record_q_scope_verification_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::QScopeVerification);
    }

    fn record_q_scope_verification_returned_scope_too_narrow_at(&mut self, at: Instant) {
        self.record_marker_event(
            at,
            EvaluatorProgressEventKind::QScopeVerificationReturnedScopeTooNarrow,
        );
    }

    fn record_marker_event(&mut self, at: Instant, kind: EvaluatorProgressEventKind) {
        self.events.push(EvaluatorProgressEvent { at, kind });
    }

    fn marker_for_window(
        &self,
        window_start: Instant,
        marker_at: Instant,
        window_kind: ProgressWindowKind,
    ) -> EvaluatorProgressMarker {
        if self.has_event_in_window(
            EvaluatorProgressEventKind::TurnTimeout,
            window_start,
            marker_at,
            window_kind,
        ) {
            return EvaluatorProgressMarker::TurnTimeout;
        }
        if window_kind == ProgressWindowKind::CompletedFullMinute
            && self.no_progress_timeout_accumulated_through_window(window_start, marker_at)
        {
            return EvaluatorProgressMarker::Idle;
        }
        for (kind, marker) in [
            (
                EvaluatorProgressEventKind::ModelFallback,
                EvaluatorProgressMarker::ModelFallback,
            ),
            (
                EvaluatorProgressEventKind::FreshThreadRetryAfterShortIdResponseError,
                EvaluatorProgressMarker::FreshThreadRetryAfterShortIdResponseError,
            ),
            (
                EvaluatorProgressEventKind::FullScopeRetry,
                EvaluatorProgressMarker::FullScopeRetry,
            ),
        ] {
            if self.has_event_in_window(kind, window_start, marker_at, window_kind) {
                return marker;
            }
        }
        // [03,w] This state belongs to one evaluated xpec, and Interrogation
        // Policy permits at most one q-scope verification follow-up for that
        // xpec. Its start and ScopeTooNarrow return therefore cannot come from
        // different verifications.
        let q_scope_verification_started = self.has_event_in_window(
            EvaluatorProgressEventKind::QScopeVerification,
            window_start,
            marker_at,
            window_kind,
        );
        let q_scope_verification_returned_scope_too_narrow = self.has_event_in_window(
            EvaluatorProgressEventKind::QScopeVerificationReturnedScopeTooNarrow,
            window_start,
            marker_at,
            window_kind,
        );
        match (
            q_scope_verification_started,
            q_scope_verification_returned_scope_too_narrow,
        ) {
            (true, true) => {
                return EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow;
            }
            (false, true) => {
                return EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow;
            }
            (true, false) => return EvaluatorProgressMarker::QScopeVerification,
            (false, false) => {}
        }
        EvaluatorProgressMarker::NoHigherPriorityEvent
    }

    fn no_progress_timeout_accumulated_through_window(
        &self,
        window_start: Instant,
        marker_at: Instant,
    ) -> bool {
        // [03] `~` means one timeout countdown remained active for the whole
        // elapsed minute. Any evaluator message resets that countdown and
        // splits the interval, so partial intervals do not make the minute `~`.
        self.active_no_progress_since
            .is_some_and(|started_at| started_at <= window_start)
            || self.completed_no_progress_intervals.iter().any(|interval| {
                interval.started_at <= window_start && interval.ended_at >= marker_at
            })
    }

    fn prune_no_progress_intervals(&mut self, marker_at: Instant) {
        self.completed_no_progress_intervals
            .retain(|interval| interval.ended_at > marker_at);
    }

    fn prune_events(&mut self, marker_at: Instant, window_kind: ProgressWindowKind) {
        self.events.retain(|event| {
            event.at > marker_at
                || (window_kind == ProgressWindowKind::CompletedFullMinute && event.at == marker_at)
        });
    }

    fn has_event_in_window(
        &self,
        kind: EvaluatorProgressEventKind,
        window_start: Instant,
        marker_at: Instant,
        window_kind: ProgressWindowKind,
    ) -> bool {
        self.events.iter().any(|event| {
            let before_window_end = match window_kind {
                ProgressWindowKind::CompletedFullMinute => event.at < marker_at,
                ProgressWindowKind::FinalMinute => event.at <= marker_at,
            };
            event.kind == kind && event.at >= window_start && before_window_end
        })
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

#[cfg(test)]
mod tests {
    use super::{EvaluatorProgress, EvaluatorProgressMarker};
    use std::time::{Duration, Instant};

    #[test] // xpec: 03
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

    #[test] // xpec: 03
    fn elapsed_marker_due_ignores_zero_interval() {
        let progress = EvaluatorProgress::new();
        let start = Instant::now();
        let mut next_marker_at = start;

        assert_eq!(
            progress
                .elapsed_marker_due(&mut next_marker_at, start, Duration::ZERO)
                .unwrap(),
            None
        );
        assert_eq!(next_marker_at, start);
    }

    #[test] // xpec: 03
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

    #[test] // xpec: 03
    fn fresh_thread_retry_after_short_id_response_error_marker_has_canon_priority() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_full_scope_retry_started_at(start + Duration::from_secs(10));
            state.record_fresh_thread_retry_after_short_id_response_error_started_at(
                start + Duration::from_secs(20),
            );
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(
            marker,
            Some(EvaluatorProgressMarker::FreshThreadRetryAfterShortIdResponseError)
        );
        assert_eq!(
            EvaluatorProgressMarker::FreshThreadRetryAfterShortIdResponseError.as_str(),
            "↻"
        );
    }

    #[test] // xpec: 03
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

    #[test] // xpec: 03
    fn q_scope_verification_scope_too_narrow_marker_uses_canon_symbol() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        progress
            .state
            .lock()
            .unwrap()
            .record_q_scope_verification_returned_scope_too_narrow_at(
                start + Duration::from_secs(30),
            );

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(
            marker,
            Some(EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow)
        );
        assert_eq!(
            EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow.as_str(),
            "↖"
        );
    }

    #[test] // xpec: 03
    fn q_scope_verification_same_window_scope_too_narrow_marker_uses_canon_symbol() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_q_scope_verification_started_at(start + Duration::from_secs(10));
            state.record_q_scope_verification_returned_scope_too_narrow_at(
                start + Duration::from_secs(30),
            );
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(
            marker,
            Some(EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow)
        );
        assert_eq!(
            EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow.as_str(),
            "⤡"
        );
    }

    #[test] // xpec: 03
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

    #[test] // xpec: E
    fn elapsed_marker_due_defers_window_end_boundary_to_the_next_minute() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let first_tick = start + interval;
        let mut next_marker_at = first_tick;

        progress
            .state
            .lock()
            .unwrap()
            .record_full_scope_retry_started_at(first_tick);

        let first = progress
            .elapsed_marker_due(&mut next_marker_at, first_tick, interval)
            .unwrap();
        let second = progress
            .elapsed_marker_due(&mut next_marker_at, first_tick + interval, interval)
            .unwrap();

        assert_eq!(first, Some(EvaluatorProgressMarker::NoHigherPriorityEvent));
        assert_eq!(second, Some(EvaluatorProgressMarker::FullScopeRetry));
    }

    #[test] // xpec: 03
    fn elapsed_marker_due_preserves_scheduled_ticks_after_late_wakeup() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_full_scope_retry_started_at(start + Duration::from_secs(70));
            state.record_q_scope_verification_started_at(start + Duration::from_secs(170));
        }

        let late_now = start + Duration::from_secs(181);
        let mut markers = Vec::new();
        while let Some(marker) = progress
            .elapsed_marker_due(&mut next_marker_at, late_now, interval)
            .unwrap()
        {
            markers.push(marker);
        }

        assert_eq!(
            markers,
            vec![
                EvaluatorProgressMarker::NoHigherPriorityEvent,
                EvaluatorProgressMarker::FullScopeRetry,
                EvaluatorProgressMarker::QScopeVerification
            ]
        );
        assert_eq!(next_marker_at, start + Duration::from_secs(240));
    }

    #[test] // xpec: 03,Od
    fn completion_markers_due_emit_overdue_idle_then_terminal_turn_timeout() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_turn_attempt_started_at(start);
            state.record_turn_timeout_at(start + Duration::from_secs(119));
            state.record_turn_attempt_finished_at(start + Duration::from_secs(119));
        }

        let markers = progress
            .completion_markers_due(
                &mut next_marker_at,
                start + Duration::from_secs(119),
                interval,
            )
            .unwrap();
        assert_eq!(
            markers,
            vec![
                EvaluatorProgressMarker::Idle,
                EvaluatorProgressMarker::TurnTimeout
            ]
        );
        assert_eq!(next_marker_at, start + Duration::from_secs(179));
    }

    #[test] // xpec: E
    fn completed_minute_timeout_has_priority_over_later_fallback() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_turn_attempt_started_at(start);
            state.record_turn_timeout_at(start + Duration::from_secs(30));
            state.record_turn_attempt_finished_at(start + Duration::from_secs(30));
            state.record_model_fallback_started_at(start + Duration::from_secs(40));
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, start + interval, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::TurnTimeout));
    }

    #[test] // xpec: 03,E,Od
    fn exact_boundary_timeout_uses_required_zero_duration_final_minute() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let timeout_at = start + Duration::from_secs(120);
        let mut next_marker_at = start + interval;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_turn_attempt_started_at(start);
            state.record_turn_timeout_at(timeout_at);
            state.record_turn_attempt_finished_at(timeout_at);
        }

        let markers = progress
            .completion_markers_due(&mut next_marker_at, timeout_at, interval)
            .unwrap();

        let elapsed_full_minutes = timeout_at.duration_since(start).as_secs() / interval.as_secs();
        assert_eq!(markers.len(), 1 + elapsed_full_minutes as usize);
        assert_eq!(
            markers,
            vec![
                EvaluatorProgressMarker::Idle,
                EvaluatorProgressMarker::Idle,
                EvaluatorProgressMarker::TurnTimeout,
            ]
        );
    }

    #[test] // xpec: 03
    fn message_activity_breaks_continuous_timeout_accumulation() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_turn_attempt_started_at(start);
            state.record_turn_message_activity_at(start + Duration::from_secs(30));
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(marker, Some(EvaluatorProgressMarker::NoHigherPriorityEvent));
    }

    #[test] // xpec: 03
    fn completion_final_marker_is_never_timeout_accumulating() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_turn_attempt_started_at(start);
            state.record_turn_attempt_finished_at(start + Duration::from_secs(30));
        }

        let markers = progress
            .completion_markers_due(
                &mut next_marker_at,
                start + Duration::from_secs(30),
                interval,
            )
            .unwrap();

        assert_eq!(
            markers,
            vec![EvaluatorProgressMarker::NoHigherPriorityEvent]
        );
    }

    #[test] // xpec: 03
    fn completion_markers_due_ignores_zero_interval() {
        let progress = EvaluatorProgress::new();
        let start = Instant::now();
        let mut next_marker_at = start;

        let markers = progress
            .completion_markers_due(&mut next_marker_at, start, Duration::ZERO)
            .unwrap();

        assert!(markers.is_empty());
        assert_eq!(next_marker_at, start);
    }

    #[test] // xpec: 03
    fn completion_markers_due_emits_overdue_window_then_final_window() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + Duration::from_secs(120);

        progress
            .state
            .lock()
            .unwrap()
            .record_turn_timeout_at(start + Duration::from_secs(125));

        let markers = progress
            .completion_markers_due(
                &mut next_marker_at,
                start + Duration::from_secs(130),
                interval,
            )
            .unwrap();

        assert_eq!(
            markers,
            vec![
                EvaluatorProgressMarker::NoHigherPriorityEvent,
                EvaluatorProgressMarker::TurnTimeout
            ]
        );
    }

    #[test] // xpec: 03
    fn completion_markers_due_emits_final_marker_for_zero_full_minutes() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;

        let markers = progress
            .completion_markers_due(
                &mut next_marker_at,
                start + Duration::from_secs(1),
                interval,
            )
            .unwrap();

        assert_eq!(
            markers,
            vec![EvaluatorProgressMarker::NoHigherPriorityEvent]
        );
        assert_eq!(next_marker_at, start + Duration::from_secs(61));
    }
}
