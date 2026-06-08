use super::program::{
    empty_tree_oid, git_head_tree_exists, resolve_tree_oid, staged_tracked_files,
    staged_tracked_files_for_pathspecs, staged_tree_oid, tree_tracked_files,
    tree_tracked_files_for_pathspecs, StagedTrackedFile,
};
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
        let tree_oid = resolve_tree_oid(root, value)
            .map_err(|err| format!("{} {}: {}", option, value, err))?;
        Ok(TreeSource::Git {
            treeish: value.to_string(),
            tree_oid,
        })
    }

    pub(crate) fn resolve_default_against_tree(
        root: &Path,
        value: &str,
        explicit: bool,
    ) -> Result<TreeSource, String> {
        validate_tree_arg(value, "--against-tree")?;
        if explicit || value != DEFAULT_AGAINST_TREE_ARG || git_head_tree_exists(root)? {
            return TreeSource::resolve(root, value, "--against-tree");
        }
        Ok(TreeSource::Git {
            treeish: value.to_string(),
            tree_oid: empty_tree_oid(root)?,
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

    pub(crate) fn tracked_files_for_pathspecs(
        &self,
        root: &Path,
        pathspecs: &[String],
    ) -> Result<Vec<StagedTrackedFile>, String> {
        match self {
            TreeSource::Staged => staged_tracked_files_for_pathspecs(root, pathspecs),
            TreeSource::Git { tree_oid, .. } => {
                tree_tracked_files_for_pathspecs(root, tree_oid, pathspecs)
            }
        }
    }

    pub(crate) fn tree_oid_for_prompt_diff(&self, root: &Path) -> Result<String, String> {
        match self {
            TreeSource::Staged => staged_tree_oid(root),
            TreeSource::Git { tree_oid, .. } => Ok(tree_oid.clone()),
        }
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
