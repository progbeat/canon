use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::EvaluatorError;

mod cwd;
mod evaluation_log;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::check::interrogation::session::thread) struct ThreadWorkspace(ThreadWorkspaceKind);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ThreadWorkspaceKind {
    InPlace,
    Git { visible_tree_oid: String },
}

impl ThreadWorkspace {
    pub(in crate::check::interrogation::session::thread) fn for_runtime(
        runtime: &CheckRuntime<'_>,
        visible_tree_oid: Option<String>,
    ) -> Result<ThreadWorkspace, EvaluatorError> {
        match (runtime.is_in_place(), visible_tree_oid) {
            (true, None) => Ok(ThreadWorkspace(ThreadWorkspaceKind::InPlace)),
            (false, Some(visible_tree_oid)) => Ok(ThreadWorkspace(ThreadWorkspaceKind::Git {
                visible_tree_oid,
            })),
            _ => Err(EvaluatorError::message(
                "check runtime produced an inconsistent evaluator workspace",
            )),
        }
    }

    #[cfg(test)]
    pub(in crate::check::interrogation::session::thread) fn in_place_for_test() -> ThreadWorkspace {
        ThreadWorkspace(ThreadWorkspaceKind::InPlace)
    }

    #[cfg(test)]
    pub(in crate::check::interrogation::session::thread) fn git_for_test(
        visible_tree_oid: &str,
    ) -> ThreadWorkspace {
        ThreadWorkspace(ThreadWorkspaceKind::Git {
            visible_tree_oid: visible_tree_oid.to_string(),
        })
    }
}
