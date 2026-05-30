use crate::config_types::AgentConfig;
use crate::git::{staged_tracked_files, tree_tracked_files, StagedTrackedFile};
use crate::hash::full_scope;
use crate::scope::{is_denied_path_bytes, path_bytes_in_scope, sanitize_scope_for_hash};
use std::path::Path;

pub(crate) const STAGED_TREE_ARG: &str = ":staged";
pub(crate) const DEFAULT_AGAINST_TREE_ARG: &str = "HEAD";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TreeSource {
    Staged,
    Git { treeish: String, tree_oid: String },
}

impl TreeSource {
    pub(crate) fn resolve(root: &Path, value: &str, option: &str) -> Result<TreeSource, String> {
        validate_tree_arg(value, option)?;
        if value == STAGED_TREE_ARG {
            return Ok(TreeSource::Staged);
        }
        let tree_oid = crate::git::resolve_tree_oid(root, value)
            .map_err(|err| format!("{} {}: {}", option, value, err))?;
        Ok(TreeSource::Git {
            treeish: value.to_string(),
            tree_oid,
        })
    }

    pub(crate) fn cache_key(&self) -> String {
        match self {
            TreeSource::Staged => STAGED_TREE_ARG.to_string(),
            TreeSource::Git { tree_oid, .. } => tree_oid.clone(),
        }
    }

    pub(crate) fn tracked_files(&self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        match self {
            TreeSource::Staged => staged_tracked_files(root),
            TreeSource::Git { tree_oid, .. } => tree_tracked_files(root, tree_oid),
        }
    }

    pub(crate) fn visible_files(
        &self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<StagedTrackedFile>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        Ok(self
            .tracked_files(root)?
            .into_iter()
            .filter(|file| file.is_blob_file_entry())
            .filter(|file| source_path_in_visible_scope(agent, &file.path, &scope))
            .collect())
    }

    pub(crate) fn is_default_checked_tree(&self) -> bool {
        matches!(self, TreeSource::Staged)
    }

    pub(crate) fn is_default_against_tree(&self) -> bool {
        matches!(self, TreeSource::Git { treeish, .. } if treeish == DEFAULT_AGAINST_TREE_ARG)
    }
}

pub(crate) fn validate_tree_arg(value: &str, option: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{} value must not be empty", option));
    }
    if value.starts_with(':') && value != STAGED_TREE_ARG {
        return Err(format!("{} unsupported pseudo-tree: {}", option, value));
    }
    Ok(())
}

fn source_path_in_visible_scope(agent: &AgentConfig, path: &[u8], scope: &[String]) -> bool {
    (scope == full_scope() || path_bytes_in_scope(path, scope))
        && !is_denied_path_bytes(agent, path)
}
