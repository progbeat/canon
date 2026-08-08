use super::super::EvaluatorProgressMarker;
use super::{EvaluatorProgressEventKind, EvaluatorProgressState, ProgressWindowKind};
use std::time::Instant;

impl EvaluatorProgressState {
    pub(in crate::evaluator::progress) fn marker_for_window(
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
        // [2gZ,qv] Multiple restricted verifications can repair one q-scope.
        // `⤡` requires the paired start and ScopeTooNarrow return of the same
        // verification in this window; adjacent verification events must not
        // be combined merely because both event kinds are present.
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
        let same_q_scope_verification_started_and_returned = self.events.iter().any(|started| {
            started.kind == EvaluatorProgressEventKind::QScopeVerification
                && self.event_is_in_window(started, window_start, marker_at, window_kind)
                && started
                    .q_scope_verification_id
                    .is_some_and(|verification_id| {
                        self.events.iter().any(|returned| {
                            returned.kind
                            == EvaluatorProgressEventKind::QScopeVerificationReturnedScopeTooNarrow
                            && returned.q_scope_verification_id == Some(verification_id)
                            && self.event_is_in_window(
                                returned,
                                window_start,
                                marker_at,
                                window_kind,
                            )
                        })
                    })
        });
        match (
            q_scope_verification_started,
            q_scope_verification_returned_scope_too_narrow,
            same_q_scope_verification_started_and_returned,
        ) {
            (_, _, true) => {
                return EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow;
            }
            (_, true, false) => {
                return EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow;
            }
            (true, false, false) => return EvaluatorProgressMarker::QScopeVerification,
            (false, false, false) => {}
        }
        EvaluatorProgressMarker::NoHigherPriorityEvent
    }

    fn no_progress_timeout_accumulated_through_window(
        &self,
        window_start: Instant,
        marker_at: Instant,
    ) -> bool {
        // [2gZ] `~` means one timeout countdown remained active for the whole
        // elapsed minute. Any evaluator message resets that countdown and
        // splits the interval, so partial intervals do not make the minute `~`.
        self.active_no_progress_since
            .is_some_and(|started_at| started_at <= window_start)
            || self.completed_no_progress_intervals.iter().any(|interval| {
                interval.started_at <= window_start && interval.ended_at >= marker_at
            })
    }

    fn has_event_in_window(
        &self,
        kind: EvaluatorProgressEventKind,
        window_start: Instant,
        marker_at: Instant,
        window_kind: ProgressWindowKind,
    ) -> bool {
        self.events.iter().any(|event| {
            event.kind == kind
                && self.event_is_in_window(event, window_start, marker_at, window_kind)
        })
    }

    fn event_is_in_window(
        &self,
        event: &super::EvaluatorProgressEvent,
        window_start: Instant,
        marker_at: Instant,
        window_kind: ProgressWindowKind,
    ) -> bool {
        let before_window_end = match window_kind {
            ProgressWindowKind::CompletedFullMinute => event.at < marker_at,
            ProgressWindowKind::FinalMinute => event.at <= marker_at,
        };
        event.at >= window_start && before_window_end
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{EvaluatorProgress, EvaluatorProgressMarker};
    use std::time::{Duration, Instant};

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
    fn adjacent_q_scope_verifications_do_not_form_same_verification_marker() {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let tick_at = start + interval;
        let mut next_marker_at = tick_at;

        {
            let mut state = progress.state.lock().unwrap();
            state.record_q_scope_verification_started_at(start - Duration::from_secs(10));
            state.record_q_scope_verification_returned_scope_too_narrow_at(
                start + Duration::from_secs(10),
            );
            state.record_q_scope_verification_started_at(start + Duration::from_secs(20));
        }

        let marker = progress
            .elapsed_marker_due(&mut next_marker_at, tick_at, interval)
            .unwrap();

        assert_eq!(
            marker,
            Some(EvaluatorProgressMarker::QScopeVerificationReturnedScopeTooNarrow)
        );
    }

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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
}
