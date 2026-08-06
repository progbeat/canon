use crate::evaluator::{
    EvaluatorProgress, EvaluatorProgressMarker, PROGRESS_TIMELINE_MARKER_INTERVAL,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// Component interface used by the live-report owner in `progress.rs`. The
// synchronization state stays private so callers and tests observe timeline
// decisions, not its representation.
pub(super) struct ElapsedProgressTimeline {
    state: Mutex<ElapsedProgressTimelineState>,
}

struct ElapsedProgressTimelineState {
    next_marker_at: Instant,
    progress_timeline: Vec<EvaluatorProgressMarker>,
    ready_to_report_at: Option<Instant>,
}

impl ElapsedProgressTimeline {
    pub(super) fn new(next_marker_at: Instant) -> ElapsedProgressTimeline {
        ElapsedProgressTimeline {
            state: Mutex::new(ElapsedProgressTimelineState {
                next_marker_at,
                progress_timeline: Vec::new(),
                ready_to_report_at: None,
            }),
        }
    }

    pub(super) fn mark_ready_to_report(&self) -> Result<(), String> {
        let mut state = self.lock_state()?;
        // [2gZ] Serialize readiness with the worker's elapsed-marker decision.
        // Once set, cleanup and thread join duration cannot extend the public
        // timeline.
        state.ready_to_report_at.get_or_insert_with(Instant::now);
        Ok(())
    }

    pub(super) fn record_progress_marker(
        &self,
        marker: EvaluatorProgressMarker,
    ) -> Result<(), String> {
        let mut state = self.lock_state()?;
        Self::record_progress_marker_in_state(&mut state, marker);
        Ok(())
    }

    fn record_progress_marker_in_state(
        state: &mut ElapsedProgressTimelineState,
        marker: EvaluatorProgressMarker,
    ) {
        state.progress_timeline.push(marker);
    }

    pub(super) fn assert_turn_timeout_has_idle_suffix(&self) -> Result<(), String> {
        let state = self.lock_state()?;
        if state.progress_timeline.last() == Some(&EvaluatorProgressMarker::TurnTimeout) {
            // [2gZ,Od] EvaluatorProgress accepts a turn-timeout marker only
            // after a continuously accumulating full marker interval. The
            // output can therefore assert its canonical suffix without
            // depending on a transport implementation detail.
            let suffix_start = state.progress_timeline.len().saturating_sub(2);
            assert_eq!(
                &state.progress_timeline[suffix_start..],
                [
                    EvaluatorProgressMarker::Idle,
                    EvaluatorProgressMarker::TurnTimeout,
                ],
                "progress_timeline[-2:] == \"~×\" for no-progress turn timeout"
            );
        }
        Ok(())
    }

    pub(super) fn rendered_progress_timeline(&self) -> Result<String, String> {
        let state = self.lock_state()?;
        Ok(state
            .progress_timeline
            .iter()
            .map(|marker| marker.as_str())
            .collect())
    }

    pub(super) fn take_elapsed_progress_marker_due(
        &self,
        progress: &EvaluatorProgress,
        now: Instant,
    ) -> Result<Option<EvaluatorProgressMarker>, String> {
        let mut state = self.lock_state()?;
        if state.ready_to_report_at.is_some() {
            return Ok(None);
        }
        let marker = progress.elapsed_marker_due(
            &mut state.next_marker_at,
            now,
            PROGRESS_TIMELINE_MARKER_INTERVAL,
        )?;
        if let Some(marker) = marker {
            // [2gZ] Taking a due elapsed marker also records its timeline
            // position. Readiness therefore cannot interleave between those
            // two decisions.
            Self::record_progress_marker_in_state(&mut state, marker);
        }
        Ok(marker)
    }

    pub(super) fn due_final_progress_markers(
        &self,
        progress: &EvaluatorProgress,
    ) -> Result<Vec<EvaluatorProgressMarker>, String> {
        let mut state = self.lock_state()?;
        let ready_to_report_at = state
            .ready_to_report_at
            .ok_or_else(|| "check live report was not marked ready".to_string())?;
        progress.completion_markers_due(
            &mut state.next_marker_at,
            ready_to_report_at,
            PROGRESS_TIMELINE_MARKER_INTERVAL,
        )
    }

    pub(super) fn wait_for_next_elapsed_marker(&self) -> Duration {
        self.lock_state()
            .map(|state| {
                state
                    .next_marker_at
                    .saturating_duration_since(Instant::now())
            })
            .unwrap_or(PROGRESS_TIMELINE_MARKER_INTERVAL)
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, ElapsedProgressTimelineState>, String> {
        self.state
            .lock()
            .map_err(|_| "check live report progress state poisoned".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 2gZ
    fn ready_timestamp_prevents_cleanup_from_extending_the_timeline() {
        let progress = EvaluatorProgress::new();
        let timeline_start = Instant::now();
        let timeline =
            ElapsedProgressTimeline::new(timeline_start + PROGRESS_TIMELINE_MARKER_INTERVAL);

        timeline.mark_ready_to_report().unwrap();
        let after_slow_cleanup = timeline_start + 2 * PROGRESS_TIMELINE_MARKER_INTERVAL;

        assert_eq!(
            timeline
                .take_elapsed_progress_marker_due(&progress, after_slow_cleanup)
                .unwrap(),
            None
        );
        assert_eq!(
            timeline.due_final_progress_markers(&progress).unwrap(),
            vec![EvaluatorProgressMarker::NoHigherPriorityEvent]
        );
    }

    #[test] // xpec: 2gZ
    fn non_timeout_timeline_has_no_timeout_suffix_requirement() {
        let timeline = ElapsedProgressTimeline::new(Instant::now());

        timeline
            .record_progress_marker(EvaluatorProgressMarker::NoHigherPriorityEvent)
            .unwrap();

        timeline.assert_turn_timeout_has_idle_suffix().unwrap();
    }

    #[test] // xpec: 2gZ,Od
    fn turn_timeout_accepts_its_required_idle_suffix() {
        let timeline = ElapsedProgressTimeline::new(Instant::now());

        timeline
            .record_progress_marker(EvaluatorProgressMarker::Idle)
            .unwrap();
        timeline
            .record_progress_marker(EvaluatorProgressMarker::TurnTimeout)
            .unwrap();

        timeline.assert_turn_timeout_has_idle_suffix().unwrap();
    }

    #[test] // xpec: 2gZ
    fn recorded_markers_render_the_complete_timeline() {
        let timeline = ElapsedProgressTimeline::new(Instant::now());

        timeline
            .record_progress_marker(EvaluatorProgressMarker::FullScopeRetry)
            .unwrap();
        timeline
            .record_progress_marker(EvaluatorProgressMarker::NoHigherPriorityEvent)
            .unwrap();

        assert_eq!(timeline.rendered_progress_timeline().unwrap(), "↗.");
    }
}
