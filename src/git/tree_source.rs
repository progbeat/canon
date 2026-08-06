use super::program::{
    staged_tracked_files_for_pathspecs, staged_tree_oid, tree_tracked_files_for_pathspecs,
    tree_tracked_files_for_pathspecs_in_environment, TrackedFile,
};
use super::GitPromptObjectArtifacts;
use std::ffi::OsString;
use std::path::Path;

pub(crate) const STAGED_TREE_ARG: &str = ":staged";
pub(crate) const DEFAULT_AGAINST_TREE_ARG: &str = "HEAD";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
// A parsed tree argument. Git-backed command preparation replaces the symbolic
// `Staged` variant with one of the OID-backed variants before downstream use.
pub(crate) enum TreeSource {
    Staged,
    Git {
        tree_oid: String,
    },
    TemporaryGit {
        tree_oid: String,
        environment: Vec<(OsString, OsString)>,
    },
    DefaultAgainstHead {
        tree_oid: String,
    },
    DefaultAgainstUnbornHead {
        empty_tree_oid: String,
    },
}

impl TreeSource {
    pub(crate) fn resolve_with(
        root: &Path,
        value: &str,
        option: &str,
        resolve_oid: impl FnOnce(&Path, &str) -> Result<Option<String>, String>,
    ) -> Result<TreeSource, String> {
        validate_tree_arg(value, option)?;
        if value == STAGED_TREE_ARG {
            return Ok(TreeSource::Staged);
        }
        let tree_oid = resolve_oid(root, value)
            .map_err(|err| format!("{} {}: {}", option, value, err))?
            .ok_or_else(|| {
                format!(
                    "{} {}: not a valid Git tree (git rev-parse failed)",
                    option, value
                )
            })?;
        Ok(TreeSource::Git { tree_oid })
    }

    pub(crate) fn resolve_default_against_with(
        root: &Path,
        value: &str,
        resolve_oid: impl FnOnce(&Path, &str) -> Result<Option<String>, String>,
        resolve_empty_tree_oid: impl FnOnce(&Path) -> Result<String, String>,
    ) -> Result<TreeSource, String> {
        validate_tree_arg(value, "--against-tree")?;
        if value != DEFAULT_AGAINST_TREE_ARG {
            return TreeSource::resolve_with(root, value, "--against-tree", resolve_oid);
        }
        // [w] A value equal to the command default has the canonical default
        // representation whether it came from clap's default or an explicit
        // `--against-tree HEAD`.
        match resolve_oid(root, value)
            .map_err(|err| format!("--against-tree {}: {}", value, err))?
        {
            Some(tree_oid) => Ok(TreeSource::DefaultAgainstHead { tree_oid }),
            None => Ok(TreeSource::DefaultAgainstUnbornHead {
                empty_tree_oid: resolve_empty_tree_oid(root)?,
            }),
        }
    }

    pub(crate) fn cache_key(&self) -> String {
        match self {
            TreeSource::Staged => STAGED_TREE_ARG.to_string(),
            TreeSource::Git { tree_oid }
            | TreeSource::TemporaryGit { tree_oid, .. }
            | TreeSource::DefaultAgainstHead { tree_oid } => tree_oid.clone(),
            TreeSource::DefaultAgainstUnbornHead { empty_tree_oid } => empty_tree_oid.clone(),
        }
    }

    pub(crate) fn tracked_files(&self, root: &Path) -> Result<Vec<TrackedFile>, String> {
        self.tracked_files_for_pathspecs(root, &[])
    }

    pub(crate) fn tracked_files_for_pathspecs(
        &self,
        root: &Path,
        pathspecs: &[String],
    ) -> Result<Vec<TrackedFile>, String> {
        match self {
            TreeSource::Staged => staged_tracked_files_for_pathspecs(root, pathspecs),
            TreeSource::Git { tree_oid } | TreeSource::DefaultAgainstHead { tree_oid } => {
                tree_tracked_files_for_pathspecs(root, tree_oid, pathspecs)
            }
            TreeSource::TemporaryGit {
                tree_oid,
                environment,
            } => tree_tracked_files_for_pathspecs_in_environment(
                root,
                tree_oid,
                pathspecs,
                environment,
            ),
            TreeSource::DefaultAgainstUnbornHead { .. } => Ok(Vec::new()),
        }
    }

    pub(crate) fn resolved_tree_oid(&self) -> Result<&str, String> {
        match self {
            TreeSource::Staged => {
                Err("symbolic staged tree reached an OID-only command phase".to_string())
            }
            TreeSource::Git { tree_oid }
            | TreeSource::TemporaryGit { tree_oid, .. }
            | TreeSource::DefaultAgainstHead { tree_oid } => Ok(tree_oid),
            TreeSource::DefaultAgainstUnbornHead { empty_tree_oid } => Ok(empty_tree_oid),
        }
    }

    pub(crate) fn tree_oid_for_prompt_diff(&self, root: &Path) -> Result<String, String> {
        self.tree_oid_resolving_staged_with(|| staged_tree_oid(root))
    }

    pub(crate) fn tree_oid_for_temporary_prompt_diff(
        &self,
        root: &Path,
        artifacts: &GitPromptObjectArtifacts,
    ) -> Result<String, String> {
        self.tree_oid_resolving_staged_with(|| artifacts.staged_tree_oid(root))
    }

    fn tree_oid_resolving_staged_with(
        &self,
        resolve_staged: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        match self {
            TreeSource::Staged => resolve_staged(),
            oid_backed => oid_backed.resolved_tree_oid().map(str::to_owned),
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
