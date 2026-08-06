//! Identifies the repository view used while expanding check configuration.

use crate::check::config::foreach_paths::expand_foreach_paths_from_listing;
use crate::check::config::foreach_paths::resolve_foreach_read_path;
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

#[derive(Clone)]
pub(crate) enum CheckConfigSource {
    Tree(TreeSource),
    InPlace,
}

impl CheckConfigSource {
    pub(crate) fn foreach_paths(
        &self,
        cache: &mut RepoInspectionCache,
        root: &Path,
        config_path: &Path,
        glob: &str,
    ) -> Result<Vec<String>, String> {
        let paths = match self {
            CheckConfigSource::Tree(source) => cache.tree_blob_paths(root, source)?,
            CheckConfigSource::InPlace => cache.in_place_file_paths(root)?,
        };
        expand_foreach_paths_from_listing(config_path, glob, &paths)
    }

    pub(crate) fn file_content(
        &self,
        cache: &mut RepoInspectionCache,
        root: &Path,
        path: &Path,
    ) -> Result<String, String> {
        match self {
            CheckConfigSource::Tree(source) => cache.tree_file_content(root, source, path),
            CheckConfigSource::InPlace => cache.in_place_file_content(root, path),
        }
    }

    pub(crate) fn foreach_literal_file_content(
        &self,
        cache: &mut RepoInspectionCache,
        root: &Path,
        config_path: &Path,
        path: &str,
    ) -> Result<String, String> {
        let path = resolve_foreach_read_path(config_path, path)?;
        self.file_content(cache, root, Path::new(&path))
    }
}
