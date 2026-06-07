use super::StagedWorktreeView;
use crate::git::TreeSource;
use crate::platform;
use crate::staged::paths::create_snapshot_root;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl StagedWorktreeView {
    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
    ) -> Result<StagedWorktreeView, String> {
        let files = source.tracked_files(root)?;
        let snapshot_root = create_snapshot_root(root)?;
        let materialization_root = snapshot_root.path().to_path_buf();
        let remove_materialization_root_on_drop = snapshot_root.remove_on_drop();
        if let Err(err) = platform::create_private_dir_all(&materialization_root.join("lazy"))
            .and_then(|_| platform::create_private_dir_all(&materialization_root.join("trees")))
        {
            if remove_materialization_root_on_drop {
                let _ = fs::remove_dir_all(&materialization_root);
            }
            return Err(format!(
                "failed to initialize evaluator materialization root {}: {}",
                materialization_root.display(),
                err
            ));
        }
        Ok(StagedWorktreeView {
            source_root: root.to_path_buf(),
            source,
            remove_materialization_root_on_drop,
            lazy_tree_dir: materialization_root.join("lazy"),
            trees_dir: materialization_root.join("trees"),
            materialization_root,
            source_files: RefCell::new(Some(files)),
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
        })
    }

    pub(super) fn source_files(&self) -> Result<Vec<crate::git::StagedTrackedFile>, String> {
        if let Some(files) = self.source_files.borrow().as_ref() {
            return Ok(files.clone());
        }
        let files = self.source.tracked_files(&self.source_root)?;
        *self.source_files.borrow_mut() = Some(files.clone());
        Ok(files)
    }
}
