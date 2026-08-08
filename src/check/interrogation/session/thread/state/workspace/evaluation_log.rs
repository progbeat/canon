use super::{ThreadWorkspace, ThreadWorkspaceKind};
use crate::check::interrogation::session::thread::ThreadTurnRequest;
use crate::evaluator::{EvaluatorError, EvaluatorPromptMode, ThreadEvaluationLogContext};
use crate::scope::effective_ignore_patterns;

impl ThreadWorkspace {
    pub(in crate::check::interrogation::session::thread) fn evaluation_log_context(
        &self,
        request: &ThreadTurnRequest<'_>,
        prompt_mode: EvaluatorPromptMode<'_>,
    ) -> Result<ThreadEvaluationLogContext, EvaluatorError> {
        let (in_place, visible_tree_oid, diff_base_tree_oid, checked_tree_oid) =
            match (&self.0, prompt_mode.git_diff_tree_oids()) {
                (ThreadWorkspaceKind::InPlace, None) => (true, None, None, None),
                (
                    ThreadWorkspaceKind::Git { visible_tree_oid },
                    Some((diff_base_tree_oid, checked_tree_oid)),
                ) => (
                    false,
                    Some(visible_tree_oid.clone()),
                    Some(diff_base_tree_oid.to_string()),
                    Some(checked_tree_oid.to_string()),
                ),
                _ => {
                    return Err(EvaluatorError::message(
                        "thread workspace and instruction views are inconsistent",
                    ));
                }
            };
        Ok(ThreadEvaluationLogContext {
            in_place,
            visible_tree_oid,
            diff_base_tree_oid,
            checked_tree_oid,
            task_input: request.task_input.to_string(),
            question_context: request.question_context.to_string(),
            plugins: request.agent.plugins.clone(),
            ignore: effective_ignore_patterns(request.agent).map_err(EvaluatorError::message)?,
        })
    }
}
