mod extract;
mod materialize;
mod path;
mod permissions;
mod setup;
#[cfg(all(test, unix))]
mod tests;
mod visible_tree;

use crate::git::{GitBlobReader, StagedTrackedFile, TreeSource};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) struct StagedWorktreeView {
    source_root: PathBuf,
    source: TreeSource,
    materialization_root: PathBuf,
    remove_materialization_root_on_drop: bool,
    lazy_tree_dir: PathBuf,
    trees_dir: PathBuf,
    source_files: RefCell<Option<Vec<StagedTrackedFile>>>,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        if self.remove_materialization_root_on_drop {
            let _ = permissions::make_materialization_tree_private(&self.trees_dir);
            let _ = std::fs::remove_dir_all(&self.materialization_root);
        }
    }
}
