//! Cached repository inspection behind a tree-independent interface.
//!
//! `files` defines inspection results, while `in_place` and `tree_source`
//! provide the two backing sources consumed through this component.

mod files;
mod in_place;
mod tree_source;

use crate::git::TrackedFile;
use crate::memoize::{mutex_memoized_result, MemoizedResult};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use in_place::{
    file_content as in_place_file_content_from_fs, file_listing as in_place_file_listing_from_fs,
};

type InPlaceFileContentCacheKey = (PathBuf, PathBuf);
type StagedFileContentCacheKey = (PathBuf, PathBuf);
type TreeFileContentCacheKey = (PathBuf, String, PathBuf);
type SourceFilesCacheKey = (PathBuf, String);
type SourcePathspecFilesCacheKey = (PathBuf, String, Vec<String>);
type GitOidAbbreviationCacheKey = (PathBuf, String);
type GitObjectExistenceCacheKey = (PathBuf, String);
type GitTreeOidCacheKey = (PathBuf, String);

// [d] One instance is the repository-input snapshot for one high-level
// command. These maps memoize repository listings and resolutions as well as
// filesystem and Git-backed file reads. Their key types above separate every
// affecting root, resolved source, path, and pathspec input. The first
// inspection fixes each mutable source's value for that command; later
// external mutations are inputs to a future command, not cache-key inputs
// inside this snapshot. Clones deliberately share that same boundary.
#[derive(Clone, Default)]
pub(crate) struct RepoInspectionCache {
    state: Arc<Mutex<RepoInspectionCacheState>>,
}

#[derive(Default)]
struct RepoInspectionCacheState {
    in_place_file_contents: BTreeMap<InPlaceFileContentCacheKey, MemoizedResult<String>>,
    staged_file_contents: BTreeMap<StagedFileContentCacheKey, MemoizedResult<String>>,
    tree_file_contents: BTreeMap<TreeFileContentCacheKey, MemoizedResult<String>>,
    staged_files: BTreeMap<PathBuf, MemoizedResult<Vec<TrackedFile>>>,
    tree_files: BTreeMap<SourceFilesCacheKey, MemoizedResult<Vec<TrackedFile>>>,
    pathspec_files: BTreeMap<SourcePathspecFilesCacheKey, MemoizedResult<Vec<TrackedFile>>>,
    in_place_files: BTreeMap<PathBuf, MemoizedResult<Vec<Vec<u8>>>>,
    git_oid_abbreviations: BTreeMap<GitOidAbbreviationCacheKey, MemoizedResult<String>>,
    git_tree_object_existence: BTreeMap<GitObjectExistenceCacheKey, MemoizedResult<bool>>,
    git_tree_oids: BTreeMap<GitTreeOidCacheKey, MemoizedResult<Option<String>>>,
    staged_tree_oids: BTreeMap<PathBuf, MemoizedResult<String>>,
    empty_tree_oids: BTreeMap<PathBuf, MemoizedResult<String>>,
}

fn cached_inspection<K: Ord, T: Clone>(
    state: &Arc<Mutex<RepoInspectionCacheState>>,
    key: K,
    map: impl for<'a> Fn(&'a RepoInspectionCacheState) -> &'a BTreeMap<K, MemoizedResult<T>>,
    map_mut: impl for<'a> Fn(&'a mut RepoInspectionCacheState) -> &'a mut BTreeMap<K, MemoizedResult<T>>,
    compute: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    mutex_memoized_result(
        state,
        key,
        "repository inspection cache lock is poisoned",
        map,
        map_mut,
        compute,
    )
}

impl RepoInspectionCache {
    pub(crate) fn new() -> RepoInspectionCache {
        RepoInspectionCache::default()
    }
}

#[cfg(test)]
mod tests;
