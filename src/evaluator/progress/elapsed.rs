use super::{EvaluatorProgress, EvaluatorProgressMarker};
use crate::evaluator::progress::state::ProgressWindowKind;
use std::time::{Duration, Instant};

impl EvaluatorProgress {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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
}
