use super::program::{
    compute_empty_tree_oid, git_head_tree_exists, resolve_tree_oid,
    staged_tracked_files_for_pathspecs, staged_tree_oid, tree_tracked_files_for_pathspecs,
    StagedTrackedFile,
};
use super::GitPromptObjectArtifacts;
use std::path::Path;

pub(crate) const STAGED_TREE_ARG: &str = ":staged";
pub(crate) const DEFAULT_AGAINST_TREE_ARG: &str = "HEAD";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TreeSource {
    Staged,
    Git { tree_oid: String },
    DefaultAgainstHead { tree_oid: String },
}

impl TreeSource {
    pub(crate) fn resolve(root: &Path, value: &str, option: &str) -> Result<TreeSource, String> {
        validate_tree_arg(value, option)?;
        if value == STAGED_TREE_ARG {
            return Ok(TreeSource::Staged);
        }
        let tree_oid = resolve_tree_oid(root, value)
            .map_err(|err| format!("{} {}: {}", option, value, err))?;
        Ok(TreeSource::Git { tree_oid })
    }

    pub(crate) fn resolve_default_against_tree(
        root: &Path,
        value: &str,
    ) -> Result<TreeSource, String> {
        validate_tree_arg(value, "--against-tree")?;
        if value != DEFAULT_AGAINST_TREE_ARG {
            return TreeSource::resolve(root, value, "--against-tree");
        }
        // [hJ] A value equal to the command default has the canonical default
        // representation whether it came from clap's default or an explicit
        // `--against-tree HEAD`.
        let tree_oid = if git_head_tree_exists(root)? {
            resolve_tree_oid(root, value)
                .map_err(|err| format!("--against-tree {}: {}", value, err))?
        } else {
            compute_empty_tree_oid(root)?
        };
        Ok(TreeSource::DefaultAgainstHead { tree_oid })
    }

    pub(crate) fn cache_key(&self) -> String {
        match self {
            TreeSource::Staged => STAGED_TREE_ARG.to_string(),
            TreeSource::Git { tree_oid } | TreeSource::DefaultAgainstHead { tree_oid } => {
                tree_oid.clone()
            }
        }
    }

    pub(crate) fn tracked_files(&self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        self.tracked_files_for_pathspecs(root, &[])
    }

    pub(crate) fn tracked_files_for_pathspecs(
        &self,
        root: &Path,
        pathspecs: &[String],
    ) -> Result<Vec<StagedTrackedFile>, String> {
        match self {
            TreeSource::Staged => staged_tracked_files_for_pathspecs(root, pathspecs),
            TreeSource::Git { tree_oid } | TreeSource::DefaultAgainstHead { tree_oid } => {
                tree_tracked_files_for_pathspecs(root, tree_oid, pathspecs)
            }
        }
    }

    pub(crate) fn tree_oid_for_prompt_diff(&self, root: &Path) -> Result<String, String> {
        match self {
            TreeSource::Staged => staged_tree_oid(root),
            TreeSource::Git { tree_oid } | TreeSource::DefaultAgainstHead { tree_oid } => {
                Ok(tree_oid.clone())
            }
        }
    }

    pub(crate) fn tree_oid_for_temporary_prompt_diff(
        &self,
        root: &Path,
        artifacts: &GitPromptObjectArtifacts,
    ) -> Result<String, String> {
        match self {
            TreeSource::Staged => artifacts.staged_tree_oid(root),
            TreeSource::Git { tree_oid } | TreeSource::DefaultAgainstHead { tree_oid } => {
                Ok(tree_oid.clone())
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{TreeSource, DEFAULT_AGAINST_TREE_ARG};
    use std::fs;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: hJ
    fn head_value_uses_default_against_head_representation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-default-against-head-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let initialized = Command::new("git").arg("init").arg(&root).output().unwrap();
        assert!(initialized.status.success());

        let source =
            TreeSource::resolve_default_against_tree(&root, DEFAULT_AGAINST_TREE_ARG).unwrap();

        assert!(matches!(source, TreeSource::DefaultAgainstHead { .. }));
        fs::remove_dir_all(root).unwrap();
    }
}
