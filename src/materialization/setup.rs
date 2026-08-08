use super::{
    root::{
        create_project_input_root, create_rollback_project_input_root, MaterializedProjectInputRoot,
    },
    TreeMaterializer,
};
use crate::git::TreeSource;
use crate::platform::filesystem::{self, PrivateTemporaryDirectoryAllocator};
use crate::repo_inspection::RepoInspectionCache;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl TreeMaterializer {
    #[cfg(test)]
    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
    ) -> Result<TreeMaterializer, String> {
        Self::apply_for_tree_source_with_repo_inspection_cache(
            root,
            source,
            RepoInspectionCache::new(),
            &PrivateTemporaryDirectoryAllocator::new(),
        )
    }

    pub(crate) fn apply_for_tree_source_with_repo_inspection_cache(
        root: &Path,
        source: TreeSource,
        repo_inspection: RepoInspectionCache,
        temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
    ) -> Result<TreeMaterializer, String> {
        Self::apply_with_project_input_root(
            root,
            source,
            repo_inspection,
            create_project_input_root(temporary_directory_allocator)?,
        )
    }

    pub(crate) fn apply_temporary_query_for_tree_source_with_repo_inspection_cache(
        root: &Path,
        source: TreeSource,
        repo_inspection: RepoInspectionCache,
        temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
    ) -> Result<TreeMaterializer, String> {
        Self::apply_with_project_input_root(
            root,
            source,
            repo_inspection,
            create_rollback_project_input_root(temporary_directory_allocator)?,
        )
    }

    fn apply_with_project_input_root(
        root: &Path,
        source: TreeSource,
        repo_inspection: RepoInspectionCache,
        project_input_root: MaterializedProjectInputRoot,
    ) -> Result<TreeMaterializer, String> {
        let input_root_path = project_input_root.tmp_dir().to_path_buf();
        let restores_caller_root_on_drop = project_input_root.restores_caller_root_on_drop();
        // [1t,g2] Keep the hardlink policy's on-disk names for tree-cache
        // compatibility. These files are the checked project input consumed
        // by the evaluator, not Canon command state. Rollback bookkeeping
        // remains in the in-memory journal below.
        let extracted_files_dir = input_root_path.join("lazy");
        let visible_trees_dir = input_root_path.join("trees");
        let extracted_files_dir_created =
            restores_caller_root_on_drop && !extracted_files_dir.is_dir();
        let visible_trees_dir_created = restores_caller_root_on_drop && !visible_trees_dir.is_dir();
        if let Err(err) = filesystem::create_private_dir_all(&extracted_files_dir)
            .and_then(|_| filesystem::create_private_dir_all(&visible_trees_dir))
        {
            if restores_caller_root_on_drop {
                if visible_trees_dir_created {
                    let _ = fs::remove_dir_all(&visible_trees_dir);
                }
                if extracted_files_dir_created {
                    let _ = fs::remove_dir_all(&extracted_files_dir);
                }
            }
            return Err(format!(
                "failed to initialize materialized evaluator input {}: {}",
                input_root_path.display(),
                err
            ));
        }
        Ok(TreeMaterializer {
            source_root: root.to_path_buf(),
            source,
            repo_inspection: RefCell::new(repo_inspection),
            materialized_input: super::MaterializedProjectInput {
                extracted_files_dir,
                visible_trees_dir,
                rollback_journal: restores_caller_root_on_drop.then(|| {
                    RefCell::new(super::MaterializationRollbackJournal {
                        extracted_files_dir_created,
                        visible_trees_dir_created,
                        extracted_path_changes: Vec::new(),
                        created_visible_tree_roots: BTreeSet::new(),
                    })
                }),
                root: project_input_root,
            },
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
        })
    }
}
