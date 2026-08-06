use super::{EvaluatorConfigError, EvaluatorConfigResult};
use std::collections::BTreeMap;

mod baseline;
mod session;

pub(super) use baseline::evaluator_baseline_permissions;
pub(super) use session::{
    evaluator_resolved_state_dir_permissions, evaluator_template_artifact_permissions,
    evaluator_working_tree_read_exception,
};

pub(super) const FILESYSTEM_DENY: &str = "deny";
pub(super) const FILESYSTEM_READ: &str = "read";

pub(super) fn merge_filesystem_permissions(
    target: &mut BTreeMap<String, String>,
    source: BTreeMap<String, String>,
) -> EvaluatorConfigResult<()> {
    for (path, permission) in source {
        insert_filesystem_permission(target, path, &permission)?;
    }
    Ok(())
}

pub(super) fn insert_filesystem_permission(
    permissions: &mut BTreeMap<String, String>,
    path: String,
    permission: &str,
) -> EvaluatorConfigResult<()> {
    if let Some(existing) = permissions.get(&path) {
        return Err(EvaluatorConfigError::DuplicateFilesystemPermission {
            path,
            existing: existing.clone(),
            replacement: permission.to_string(),
        });
    }
    permissions.insert(path, permission.to_string());
    Ok(())
}

fn single_filesystem_permission(path: String, permission: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(path, permission.to_string())])
}
