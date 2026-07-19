use super::path::relative_path_from_git_path;
use super::permissions::remove_write_permissions_from_materialized_dir;
use super::visible_tree::VisibleTree;
use super::StagedWorktreeView;
use crate::platform;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

// Implements the hardlink materialization policy's materialize() step.
// setup.rs owns tmp_dir/lazy_tree_dir/trees_dir/unpacked_paths initialization;
// extract.rs unpacks missing visible entries into lazy_tree_dir and removes
// file write permissions; this module reuses trees/<visible_tree_oid> when it
// exists, hardlinks files or copies symlinks into a new tree, and removes write
// permissions from each materialized directory.
impl StagedWorktreeView {
    pub(crate) fn visible_tree_root_path(&self, visible_tree_oid: &str) -> PathBuf {
        self.trees_dir.join(visible_tree_oid)
    }

    pub(crate) fn materialize_visible_scope(
        &self,
        visible_scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        let visible_tree = self.visible_tree(visible_scope, visible_tree_oid)?;
        self.materialize_visible_tree(&visible_tree)
    }

    fn materialize_visible_tree(&self, visible_tree: &VisibleTree) -> Result<PathBuf, String> {
        let visible_tree_root = self.visible_tree_root_path(&visible_tree.oid);
        if visible_tree_root.exists() {
            return Ok(visible_tree_root);
        }
        self.unpack_missing_visible_entries(visible_tree)?;
        self.hardlink_visible_tree_root(visible_tree, visible_tree_root)
    }

    fn hardlink_visible_tree_root(
        &self,
        visible_tree: &VisibleTree,
        visible_tree_root: PathBuf,
    ) -> Result<PathBuf, String> {
        let materializing_root = self.create_materializing_tree_root(&visible_tree.oid)?;
        let built = self
            .hardlink_visible_tree_children(visible_tree, b"", &materializing_root)
            .and_then(|()| remove_write_permissions_from_materialized_dir(&materializing_root));
        if let Err(err) = built {
            return Err(cleanup_materializing_tree_root(&materializing_root, err));
        }

        // [tb] Publish only a complete, read-only tree. A concurrent invocation
        // can observe either no OID root or the finished root, never a directory
        // that this invocation is still populating.
        if let Err(err) = fs::rename(&materializing_root, &visible_tree_root) {
            if visible_tree_root.exists() {
                super::remove_materialization_path(&materializing_root).map_err(|cleanup_err| {
                    format!(
                        "failed to remove redundant evaluator visible tree {} after another \
                         invocation published {}: {}",
                        materializing_root.display(),
                        visible_tree_root.display(),
                        cleanup_err
                    )
                })?;
                return Ok(visible_tree_root);
            }
            let publish_error = format!(
                "failed to publish evaluator visible tree {} as {}: {}",
                materializing_root.display(),
                visible_tree_root.display(),
                err
            );
            return Err(cleanup_materializing_tree_root(
                &materializing_root,
                publish_error,
            ));
        }
        if let Some(journal) = &self.invocation_journal {
            journal
                .borrow_mut()
                .created_visible_tree_roots
                .insert(visible_tree_root.clone());
        }
        Ok(visible_tree_root)
    }

    fn create_materializing_tree_root(&self, visible_tree_oid: &str) -> Result<PathBuf, String> {
        for _ in 0..64 {
            let random = getrandom::u64()
                .map_err(|err| format!("failed to choose visible tree staging path: {err}"))?;
            let path = self.trees_dir.join(format!(
                ".canon-materializing-{}-{visible_tree_oid}-{random:016x}",
                std::process::id()
            ));
            match platform::create_private_dir(&path) {
                Ok(()) => return Ok(path),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(format!(
                        "failed to create evaluator visible tree staging root {}: {}",
                        path.display(),
                        err
                    ));
                }
            }
        }
        Err(format!(
            "failed to allocate evaluator visible tree staging root under {}",
            self.trees_dir.display()
        ))
    }

    fn hardlink_visible_tree_children(
        &self,
        visible_tree: &VisibleTree,
        prefix: &[u8],
        target_dir: &Path,
    ) -> Result<(), String> {
        for child in visible_tree.children(prefix) {
            let target = target_dir.join(platform::os_string_from_bytes(child.name)?);
            if child.is_dir {
                platform::create_private_dir(&target).map_err(|err| {
                    format!(
                        "failed to create evaluator visible tree directory {}: {}",
                        target.display(),
                        err
                    )
                })?;
                self.hardlink_visible_tree_children(visible_tree, &child.path, &target)?;
                remove_write_permissions_from_materialized_dir(&target)?;
            } else {
                let source = self
                    .lazy_tree_dir
                    .join(relative_path_from_git_path(&child.path)?);
                platform::hardlink_file_or_copy_symlink(&source, &target)?;
            }
        }
        Ok(())
    }
}

fn cleanup_materializing_tree_root(path: &Path, primary_error: String) -> String {
    match super::remove_materialization_path(path) {
        Ok(()) => primary_error,
        Err(cleanup_error) => format!(
            "{}; failed to remove incomplete evaluator visible tree {}: {}",
            primary_error,
            path.display(),
            cleanup_error
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{StagedTrackedFile, TreeSource};

    #[test] // xpec: tb
    fn failed_tree_build_never_exposes_the_oid_root() {
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(Path::new("."), TreeSource::Staged).unwrap();
        let visible_tree = VisibleTree {
            oid: "0000000000000000000000000000000000000000".to_string(),
            entry_paths: vec![StagedTrackedFile {
                path: b"missing-gitlink".to_vec(),
                mode: "160000".to_string(),
                object_id: "1111111111111111111111111111111111111111".to_string(),
            }],
        };
        let visible_tree_root = staged_view.visible_tree_root_path(&visible_tree.oid);

        let error = staged_view
            .hardlink_visible_tree_root(&visible_tree, visible_tree_root.clone())
            .expect_err("missing lazy entry must fail materialization");

        assert!(error.contains("missing-gitlink"));
        assert!(!visible_tree_root.exists());
        assert!(
            fs::read_dir(&staged_view.trees_dir)
                .unwrap()
                .next()
                .is_none(),
            "failed materialization must remove its private staging tree"
        );
    }
}
