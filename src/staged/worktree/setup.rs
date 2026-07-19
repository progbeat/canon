use super::StagedWorktreeView;
use crate::git::TreeSource;
use crate::platform;
use crate::staged::paths::{
    create_invocation_local_materialization_root, create_temporary_materialization_root,
    TemporaryMaterializationRoot,
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl StagedWorktreeView {
    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
    ) -> Result<StagedWorktreeView, String> {
        let temporary_root = create_temporary_materialization_root()?;
        Self::apply_with_temporary_root(root, source, temporary_root)
    }

    pub(crate) fn apply_invocation_local_for_tree_source(
        root: &Path,
        source: TreeSource,
    ) -> Result<StagedWorktreeView, String> {
        let temporary_root = create_invocation_local_materialization_root()?;
        Self::apply_with_temporary_root(root, source, temporary_root)
    }

    #[cfg(test)]
    pub(super) fn apply_invocation_local_for_tree_source_at(
        root: &Path,
        source: TreeSource,
        configured_tmp_dir: &Path,
    ) -> Result<StagedWorktreeView, String> {
        let temporary_root =
            crate::staged::paths::create_invocation_local_materialization_root_for(Some(
                configured_tmp_dir.to_path_buf(),
            ))?;
        Self::apply_with_temporary_root(root, source, temporary_root)
    }

    fn apply_with_temporary_root(
        root: &Path,
        source: TreeSource,
        temporary_root: TemporaryMaterializationRoot,
    ) -> Result<StagedWorktreeView, String> {
        let tmp_dir = temporary_root.tmp_dir().to_path_buf();
        let canon_owns_tmp_dir = temporary_root.is_canon_owned();
        let restores_invocation_local_artifacts =
            temporary_root.restores_invocation_local_artifacts();
        let lazy_tree_dir = tmp_dir.join("lazy");
        let trees_dir = tmp_dir.join("trees");
        let lazy_tree_dir_created = restores_invocation_local_artifacts && !lazy_tree_dir.is_dir();
        let trees_dir_created = restores_invocation_local_artifacts && !trees_dir.is_dir();
        if let Err(err) = platform::create_private_dir_all(&lazy_tree_dir)
            .and_then(|_| platform::create_private_dir_all(&trees_dir))
        {
            if canon_owns_tmp_dir {
                let _ = fs::remove_dir_all(&tmp_dir);
            } else if restores_invocation_local_artifacts {
                if trees_dir_created {
                    let _ = fs::remove_dir_all(&trees_dir);
                }
                if lazy_tree_dir_created {
                    let _ = fs::remove_dir_all(&lazy_tree_dir);
                }
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
            invocation_journal: restores_invocation_local_artifacts.then(|| {
                RefCell::new(super::InvocationMaterializationJournal {
                    lazy_tree_dir_created,
                    trees_dir_created,
                    lazy_path_changes: Vec::new(),
                    created_visible_tree_roots: BTreeSet::new(),
                })
            }),
            lazy_tree_dir,
            trees_dir,
            tmp_dir,
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
        })
    }
}
