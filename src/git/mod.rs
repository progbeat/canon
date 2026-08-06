//! Shared Git data boundary.
//!
//! This component owns reusable Git configuration, repository-path and
//! tree/object resolution, prompt-only object artifacts, and scoped-tree OID
//! derivation. Components with domain-specific Git interactions own those
//! commands locally.

mod config;
mod object_database;
mod program;
mod tree_source;
mod visible_tree_oid;

pub(crate) use config::{git_config_get, GitConfigGetError};
pub(crate) use object_database::GitPromptObjectArtifacts;
pub(crate) use program::{
    abbreviate_git_oid, compute_empty_tree_oid, git_common_dir, git_project_root, is_git_worktree,
    read_git_blobs, resolve_git_path, resolve_tree_oid_if_exists, staged_tracked_files,
    tree_object_exists, GitBlobReader, TrackedFile,
};
pub(crate) use tree_source::{
    validate_tree_arg, TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG,
};
pub(crate) use visible_tree_oid::{git_object_oid_has_known_shape, VisibleTreeOidCache};
