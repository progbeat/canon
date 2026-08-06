mod marker;

use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct EvaluatorProgressState {
    events: Vec<EvaluatorProgressEvent>,
    active_no_progress_since: Option<Instant>,
    completed_no_progress_intervals: Vec<NoProgressInterval>,
    next_q_scope_verification_id: u64,
    active_q_scope_verification_id: Option<u64>,
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
    q_scope_verification_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoProgressInterval {
    started_at: Instant,
    ended_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProgressWindowKind {
    CompletedFullMinute,
    FinalMinute,
}

impl EvaluatorProgressState {
    pub(super) fn record_turn_attempt_started_at(&mut self, at: Instant) {
        self.finish_active_no_progress_interval_at(at);
        self.active_no_progress_since = Some(at);
    }

    pub(super) fn record_turn_message_activity_at(&mut self, at: Instant) {
        if self.active_no_progress_since.is_none() {
            return;
        }
        self.finish_active_no_progress_interval_at(at);
        self.active_no_progress_since = Some(at);
    }

    pub(super) fn record_turn_attempt_finished_at(&mut self, at: Instant) {
        self.finish_active_no_progress_interval_at(at);
    }

    pub(super) fn record_turn_timeout_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::TurnTimeout);
    }

    pub(super) fn assert_turn_timeout_guarantees_preceding_idle_marker(
        &self,
        at: Instant,
        marker_interval: Duration,
    ) {
        // [2gZ,Od] `TurnTimeout` is part of the progress component's public
        // marker contract. Two continuously accumulating intervals guarantee
        // a completed idle window immediately before the terminal partial
        // window regardless of the turn start's phase within the timeline.
        assert!(
            self.active_no_progress_since
                .and_then(|started_at| at.checked_duration_since(started_at))
                .is_some_and(|elapsed| elapsed >= marker_interval.saturating_mul(2)),
            "turn timeout must guarantee a preceding full idle marker"
        );
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

    pub(super) fn record_model_fallback_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::ModelFallback);
    }

    pub(super) fn record_fresh_thread_retry_after_short_id_response_error_started_at(
        &mut self,
        at: Instant,
    ) {
        self.record_marker_event(
            at,
            EvaluatorProgressEventKind::FreshThreadRetryAfterShortIdResponseError,
        );
    }

    pub(super) fn record_full_scope_retry_started_at(&mut self, at: Instant) {
        self.record_marker_event(at, EvaluatorProgressEventKind::FullScopeRetry);
    }

    pub(super) fn record_q_scope_verification_started_at(&mut self, at: Instant) {
        let verification_id = self.allocate_q_scope_verification_id();
        self.active_q_scope_verification_id = Some(verification_id);
        self.record_q_scope_verification_event(
            at,
            EvaluatorProgressEventKind::QScopeVerification,
            verification_id,
        );
    }

    pub(super) fn record_q_scope_verification_returned_scope_too_narrow_at(&mut self, at: Instant) {
        let verification_id = match self.active_q_scope_verification_id.take() {
            Some(verification_id) => verification_id,
            None => self.allocate_q_scope_verification_id(),
        };
        self.record_q_scope_verification_event(
            at,
            EvaluatorProgressEventKind::QScopeVerificationReturnedScopeTooNarrow,
            verification_id,
        );
    }

    fn allocate_q_scope_verification_id(&mut self) -> u64 {
        let verification_id = self.next_q_scope_verification_id;
        self.next_q_scope_verification_id = self.next_q_scope_verification_id.wrapping_add(1);
        verification_id
    }

    fn record_q_scope_verification_event(
        &mut self,
        at: Instant,
        kind: EvaluatorProgressEventKind,
        verification_id: u64,
    ) {
        self.events.push(EvaluatorProgressEvent {
            at,
            kind,
            q_scope_verification_id: Some(verification_id),
        });
    }

    fn record_marker_event(&mut self, at: Instant, kind: EvaluatorProgressEventKind) {
        self.events.push(EvaluatorProgressEvent {
            at,
            kind,
            q_scope_verification_id: None,
        });
    }

    pub(super) fn prune_no_progress_intervals(&mut self, marker_at: Instant) {
        self.completed_no_progress_intervals
            .retain(|interval| interval.ended_at > marker_at);
    }

    pub(super) fn prune_events(&mut self, marker_at: Instant, window_kind: ProgressWindowKind) {
        self.events.retain(|event| {
            event.at > marker_at
                || (window_kind == ProgressWindowKind::CompletedFullMinute && event.at == marker_at)
        });
    }
}
