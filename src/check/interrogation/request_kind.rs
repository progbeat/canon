use crate::evaluator::EvaluatorProgress;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterrogationRequestKind {
    Initial,
    FullScopeRetry,
    QScopeVerification,
}

impl InterrogationRequestKind {
    pub(crate) fn record_started_progress_marker(self, progress: Option<&EvaluatorProgress>) {
        let Some(progress) = progress else {
            return;
        };
        match self {
            InterrogationRequestKind::Initial => {}
            InterrogationRequestKind::FullScopeRetry => {
                progress.record_full_scope_retry_started();
            }
            InterrogationRequestKind::QScopeVerification => {
                progress.record_q_scope_verification_started();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InterrogationRequestKind;
    use crate::evaluator::{EvaluatorProgress, EvaluatorProgressMarker};
    use std::time::{Duration, Instant};

    #[test]
    fn non_initial_request_kinds_record_dedicated_markers() {
        assert_eq!(
            marker_after(InterrogationRequestKind::FullScopeRetry),
            EvaluatorProgressMarker::FullScopeRetry
        );
        assert_eq!(
            marker_after(InterrogationRequestKind::QScopeVerification),
            EvaluatorProgressMarker::QScopeVerification
        );
    }

    #[test]
    fn initial_request_kind_keeps_default_marker() {
        assert_eq!(
            marker_after(InterrogationRequestKind::Initial),
            EvaluatorProgressMarker::NoHigherPriorityEvent
        );
    }

    fn marker_after(kind: InterrogationRequestKind) -> EvaluatorProgressMarker {
        let progress = EvaluatorProgress::new();
        let interval = Duration::from_secs(60);
        let start = Instant::now();
        let mut next_marker_at = start + interval;
        kind.record_started_progress_marker(Some(&progress));
        let now = Instant::now();
        progress
            .completion_markers_due(&mut next_marker_at, now, interval)
            .unwrap()
            .remove(0)
    }
}
