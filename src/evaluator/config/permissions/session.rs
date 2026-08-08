use super::{
    single_filesystem_permission, EvaluatorConfigResult, FILESYSTEM_DENY, FILESYSTEM_READ,
};
use crate::evaluator::config::path_to_config_string;
use std::collections::BTreeMap;
use std::path::Path;

pub(in crate::evaluator::config) fn evaluator_working_tree_read_exception(
    session_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    // Grant only the evaluator session root. Adding a redundant denial for its
    // parent blocks traversal before the sandbox reaches this read exception.
    Ok(single_filesystem_permission(
        path_to_config_string(session_root, "evaluator session root")?,
        FILESYSTEM_READ,
    ))
}

pub(in crate::evaluator::config) fn evaluator_template_artifact_permissions(
    template_artifact_directory: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    tree_permission(template_artifact_directory, FILESYSTEM_READ)
}

pub(in crate::evaluator::config) fn evaluator_resolved_state_dir_permissions(
    state_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    tree_permission(state_root, FILESYSTEM_DENY)
}

fn tree_permission(
    path: &Path,
    permission: &str,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let path = path_to_config_string(path, "evaluator filesystem permission path")?;
    Ok(single_filesystem_permission(path, permission))
}
