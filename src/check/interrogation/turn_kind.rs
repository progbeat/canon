use crate::evaluator::EvaluatorProgress;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterrogationTurnKind {
    Initial,
    FullScopeRetry,
    QScopeVerification,
}

impl InterrogationTurnKind {
    pub(crate) fn record_turn_started_progress_marker(self, progress: Option<&EvaluatorProgress>) {
        self.record_progress_marker(progress, InterrogationTurnProgressEvent::Started);
    }

    pub(crate) fn record_scope_too_narrow_progress_marker(
        self,
        progress: Option<&EvaluatorProgress>,
    ) {
        self.record_progress_marker(
            progress,
            InterrogationTurnProgressEvent::ReturnedScopeTooNarrow,
        );
    }

    fn record_progress_marker(
        self,
        progress: Option<&EvaluatorProgress>,
        event: InterrogationTurnProgressEvent,
    ) {
        let Some(progress) = progress else {
            return;
        };
        match (self, event) {
            (InterrogationTurnKind::FullScopeRetry, InterrogationTurnProgressEvent::Started) => {
                progress.record_full_scope_retry_started();
            }
            (
                InterrogationTurnKind::QScopeVerification,
                InterrogationTurnProgressEvent::Started,
            ) => {
                progress.record_q_scope_verification_started();
            }
            (
                InterrogationTurnKind::QScopeVerification,
                InterrogationTurnProgressEvent::ReturnedScopeTooNarrow,
            ) => {
                progress.record_q_scope_verification_returned_scope_too_narrow();
            }
            (InterrogationTurnKind::Initial, _)
            | (
                InterrogationTurnKind::FullScopeRetry,
                InterrogationTurnProgressEvent::ReturnedScopeTooNarrow,
            ) => {}
        }
    }
}

#[derive(Clone, Copy)]
enum InterrogationTurnProgressEvent {
    Started,
    ReturnedScopeTooNarrow,
}

#[cfg(test)]
mod tests {
    use super::InterrogationTurnKind;
    use crate::evaluator::{EvaluatorProgress, EvaluatorProgressMarker};
    use std::time::{Duration, Instant};

    #[test] // xpec: 2gZ,fc
    fn non_initial_turn_kinds_record_dedicated_markers() {
        assert_eq!(
            marker_after(InterrogationTurnKind::FullScopeRetry),
            EvaluatorProgressMarker::FullScopeRetry
        );
        assert_eq!(
            marker_after(InterrogationTurnKind::QScopeVerification),
            EvaluatorProgressMarker::QScopeVerification
        );
    }

    #[test] // xpec: 2gZ
    fn initial_turn_kind_keeps_default_marker() {
        assert_eq!(
            marker_after(InterrogationTurnKind::Initial),
            EvaluatorProgressMarker::NoHigherPriorityEvent
        );
    }

    #[test] // xpec: 2gZ
    fn only_verification_records_scope_too_narrow_as_a_verification_return() {
        assert_eq!(
            marker_after_scope_too_narrow(InterrogationTurnKind::FullScopeRetry),
            EvaluatorProgressMarker::FullScopeRetry
        );
        assert_eq!(
            marker_after_scope_too_narrow(InterrogationTurnKind::QScopeVerification),
            EvaluatorProgressMarker::QScopeVerificationStartedAndReturnedScopeTooNarrow
        );
    }

    fn marker_after(kind: InterrogationTurnKind) -> EvaluatorProgressMarker {
        marker_after_events(kind, false)
    }

    fn marker_after_scope_too_narrow(kind: InterrogationTurnKind) -> EvaluatorProgressMarker {
        marker_after_events(kind, true)
    }

    fn marker_after_events(
        kind: InterrogationTurnKind,
        returned_scope_too_narrow: bool,
    ) -> EvaluatorProgressMarker {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;
        kind.record_turn_started_progress_marker(Some(&progress));
        if returned_scope_too_narrow {
            kind.record_scope_too_narrow_progress_marker(Some(&progress));
        }
        let now = Instant::now();
        progress
            .completion_markers_due(&mut next_marker_at, now, interval)
            .unwrap()
            .remove(0)
    }
}
