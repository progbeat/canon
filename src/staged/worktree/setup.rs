use super::StagedWorktreeView;
use crate::git::TreeSource;
use crate::platform;
use crate::staged::paths::{create_temporary_materialization_root, TemporaryMaterializationRoot};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl StagedWorktreeView {
    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
    ) -> Result<StagedWorktreeView, String> {
        let temporary_root = create_temporary_materialization_root(root)?;
        Self::apply_with_temporary_root(root, source, temporary_root)
    }

    fn apply_with_temporary_root(
        root: &Path,
        source: TreeSource,
        temporary_root: TemporaryMaterializationRoot,
    ) -> Result<StagedWorktreeView, String> {
        let tmp_dir = temporary_root.tmp_dir().to_path_buf();
        let canon_owns_tmp_dir = temporary_root.is_canon_owned();
        if let Err(err) = platform::create_private_dir_all(&tmp_dir.join("lazy"))
            .and_then(|_| platform::create_private_dir_all(&tmp_dir.join("trees")))
        {
            if canon_owns_tmp_dir {
                let _ = fs::remove_dir_all(&tmp_dir);
            }
            return Err(format!(
                "failed to initialize evaluator materialization root {}: {}",
                tmp_dir.display(),
                err
            ));
        }
        Ok(StagedWorktreeView {
            source_root: root.to_path_buf(),
            source,
            canon_owns_tmp_dir,
            lazy_tree_dir: tmp_dir.join("lazy"),
            trees_dir: tmp_dir.join("trees"),
            tmp_dir,
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
        })
    }
}
