use super::{single_filesystem_permission, EvaluatorConfigResult};
use crate::evaluator::config::path_to_config_string;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) fn evaluator_baseline_permissions(
    codex_executable: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    Ok(single_filesystem_permission(
        path_to_config_string(codex_executable, "codex executable")?,
        "read",
    ))
}
