//! Filesystem permission fields in the Codex runtime configuration.

use super::super::permissions::evaluator_baseline_permissions;
use super::super::{EvaluatorConfigError, EvaluatorConfigResult};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn evaluator_filesystem_config_entries(
    root_access: &str,
    extra_permissions: &BTreeMap<String, String>,
    codex_executable: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut entries = BTreeMap::new();
    insert_filesystem_config_entry(&mut entries, ":root".to_string(), root_access.to_string())?;
    insert_filesystem_config_entry(&mut entries, ":minimal".to_string(), "read".to_string())?;
    for (path, permission) in evaluator_baseline_permissions(codex_executable)? {
        insert_filesystem_config_entry(&mut entries, path, permission)?;
    }
    for (path, permission) in extra_permissions {
        insert_filesystem_config_entry(&mut entries, path.clone(), permission.clone())?;
    }
    Ok(entries)
}

fn insert_filesystem_config_entry(
    entries: &mut BTreeMap<String, String>,
    path: String,
    value: String,
) -> EvaluatorConfigResult<()> {
    if entries.contains_key(&path) {
        return Err(EvaluatorConfigError::DuplicateFilesystemConfigEntry { path });
    }
    entries.insert(path, value);
    Ok(())
}
