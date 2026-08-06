use crate::evaluator::EvaluatorProgress;

pub(super) struct ActiveTurnProgress {
    progress: Option<EvaluatorProgress>,
}

impl ActiveTurnProgress {
    pub(super) fn start(progress: Option<EvaluatorProgress>) -> ActiveTurnProgress {
        if let Some(progress) = &progress {
            progress.record_turn_attempt_started();
        }
        ActiveTurnProgress { progress }
    }
}

impl Drop for ActiveTurnProgress {
    fn drop(&mut self) {
        if let Some(progress) = &self.progress {
            progress.record_turn_attempt_finished();
        }
    }
}
