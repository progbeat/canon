mod config;
mod program;
mod tree_source;
mod visible_tree_oid;

pub(crate) use config::{git_config_get, GitConfigGetError};
pub(crate) use program::{
    abbreviate_git_oid, read_git_blobs, resolve_git_path, staged_tracked_files, tree_object_exists,
    GitBlobReader, StagedTrackedFile,
};
#[cfg(test)]
pub(crate) use program::{empty_tree_oid, staged_tree_oid};
pub(crate) use tree_source::{
    validate_tree_arg, TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG,
};
pub(crate) use visible_tree_oid::{
    git_object_oid_has_known_shape, StoredVisibleScopeOidResolver, VisibleTreeOidCache,
};
