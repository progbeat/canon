use super::super::super::AppServerRunner;
use crate::evaluator::EvaluatorError;

impl AppServerRunner {
    pub(super) fn finish_turn_usage(
        &mut self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), EvaluatorError> {
        let drain_result = self.drain_token_usage_updates();
        let turn_usage = turn_id.map(|turn_id| self.turn_usage_for_turn(thread_id, turn_id));
        if let Some(turn_usage) = &turn_usage {
            self.apply_thread_reuse_policy(turn_usage);
        }
        self.last_turn_usage = turn_usage;
        drain_result
    }
}
