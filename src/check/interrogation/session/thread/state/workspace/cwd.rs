use super::{ThreadWorkspace, ThreadWorkspaceKind};
use crate::check::interrogation::session::thread::{state::ThreadState, ThreadTurnRequest};
use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::EvaluatorError;
use std::path::PathBuf;

impl ThreadWorkspace {
    pub(in crate::check::interrogation::session::thread) fn prepare_cwd(
        &self,
        runtime: &CheckRuntime<'_>,
        thread_state: &mut ThreadState,
        request: &ThreadTurnRequest<'_>,
    ) -> Result<PathBuf, EvaluatorError> {
        let visible_tree_oid = match (&self.0, runtime.is_in_place()) {
            (ThreadWorkspaceKind::InPlace, true) => {
                // In-place mode starts the evaluator in the checked directory itself.
                // Moving it to an isolation path would change the checked directory.
                return Ok(runtime.root.to_path_buf());
            }
            (ThreadWorkspaceKind::Git { visible_tree_oid }, false) => visible_tree_oid,
            _ => {
                return Err(EvaluatorError::message(
                    "thread workspace does not match the check runtime",
                ));
            }
        };
        let canonical_root = runtime
            .materialized_session_root_path(visible_tree_oid)
            .map_err(EvaluatorError::message)?;
        thread_state
            .prepare_materialized_session_root(&canonical_root, || {
                runtime.session_root_for_scope(
                    request.agent,
                    request.enforced_scope,
                    Some(visible_tree_oid),
                )
            })
            .map_err(EvaluatorError::message)
    }
}
