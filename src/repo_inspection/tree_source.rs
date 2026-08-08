use super::{cached_inspection, RepoInspectionCache};
use crate::git::{
    abbreviate_git_oid, compute_empty_tree_oid, resolve_tree_oid_if_exists, tree_object_exists,
    TreeSource,
};
use std::path::Path;
use std::sync::Arc;

impl RepoInspectionCache {
    pub(crate) fn resolve_tree(
        &mut self,
        root: &Path,
        value: &str,
        option: &str,
    ) -> Result<TreeSource, String> {
        let state = Arc::clone(&self.state);
        TreeSource::resolve_with(root, value, option, |root, value| {
            cached_inspection(
                &state,
                (root.to_path_buf(), value.to_string()),
                |state| &state.git_tree_oids,
                |state| &mut state.git_tree_oids,
                || resolve_tree_oid_if_exists(root, value),
            )
        })
    }

    // [Tv] Commands that do not need an invocation-private object database use
    // this preparation boundary to freeze `:staged` once and consume only its
    // OID-backed tree afterward.
    pub(crate) fn resolve_tree_to_oid_source(
        &mut self,
        root: &Path,
        value: &str,
        option: &str,
    ) -> Result<TreeSource, String> {
        let source = self.resolve_tree(root, value, option)?;
        if !matches!(source, TreeSource::Staged) {
            return Ok(source);
        }
        let tree_oid = cached_inspection(
            &self.state,
            root.to_path_buf(),
            |state| &state.staged_tree_oids,
            |state| &mut state.staged_tree_oids,
            || source.tree_oid_for_prompt_diff(root),
        )?;
        Ok(TreeSource::Git { tree_oid })
    }

    pub(crate) fn resolve_default_against_tree(
        &mut self,
        root: &Path,
        value: &str,
    ) -> Result<TreeSource, String> {
        let tree_state = Arc::clone(&self.state);
        let empty_state = Arc::clone(&self.state);
        TreeSource::resolve_default_against_with(
            root,
            value,
            |root, value| {
                cached_inspection(
                    &tree_state,
                    (root.to_path_buf(), value.to_string()),
                    |state| &state.git_tree_oids,
                    |state| &mut state.git_tree_oids,
                    || resolve_tree_oid_if_exists(root, value),
                )
            },
            |root| {
                cached_inspection(
                    &empty_state,
                    root.to_path_buf(),
                    |state| &state.empty_tree_oids,
                    |state| &mut state.empty_tree_oids,
                    || compute_empty_tree_oid(root),
                )
            },
        )
    }

    pub(crate) fn resolve_checked_and_against_trees(
        &mut self,
        root: &Path,
        checked_tree: &str,
        against_tree: &str,
    ) -> Result<(TreeSource, TreeSource), String> {
        let checked_tree = self.resolve_tree(root, checked_tree, "--tree")?;
        let against_tree = self.resolve_default_against_tree(root, against_tree)?;
        Ok((checked_tree, against_tree))
    }

    pub(crate) fn git_oid_abbreviation(
        &mut self,
        root: &Path,
        oid: &str,
    ) -> Result<String, String> {
        let key = (root.to_path_buf(), oid.to_string());
        cached_inspection(
            &self.state,
            key,
            |state| &state.git_oid_abbreviations,
            |state| &mut state.git_oid_abbreviations,
            || abbreviate_git_oid(root, oid),
        )
    }

    pub(crate) fn git_tree_object_exists(
        &mut self,
        root: &Path,
        oid: &str,
    ) -> Result<bool, String> {
        let key = (root.to_path_buf(), oid.to_string());
        cached_inspection(
            &self.state,
            key,
            |state| &state.git_tree_object_existence,
            |state| &mut state.git_tree_object_existence,
            || tree_object_exists(root, oid),
        )
    }
}
