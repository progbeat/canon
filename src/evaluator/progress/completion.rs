use super::{EvaluatorProgress, EvaluatorProgressMarker};
use crate::evaluator::progress::state::ProgressWindowKind;
use std::time::{Duration, Instant};

impl EvaluatorProgress {
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
        // [2gZ] The loop emitted every completed full minute as a half-open
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 2gZ,Od
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

    #[test] // xpec: 2gZ,Od
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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

    #[test] // xpec: 2gZ
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
