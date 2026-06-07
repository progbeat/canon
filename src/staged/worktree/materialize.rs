use super::path::relative_path_from_git_path;
use super::permissions::remove_write_permissions_from_materialized_dir;
use super::visible_tree::VisibleTree;
use super::StagedWorktreeView;
use crate::platform;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

impl StagedWorktreeView {
    pub(crate) fn materialize_visible_scope(
        &self,
        visible_scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        let visible_tree = self.visible_tree(visible_scope, visible_tree_oid)?;
        self.materialize_visible_tree(&visible_tree)
    }

    fn materialize_visible_tree(&self, visible_tree: &VisibleTree) -> Result<PathBuf, String> {
        let visible_tree_root = self.trees_dir.join(&visible_tree.oid);
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
        if let Err(err) = platform::create_private_dir(&visible_tree_root) {
            if err.kind() == ErrorKind::AlreadyExists {
                return Ok(visible_tree_root);
            }
            return Err(format!(
                "failed to create evaluator visible tree root {}: {}",
                visible_tree_root.display(),
                err
            ));
        }
        self.hardlink_visible_tree_children(visible_tree, b"", &visible_tree_root)?;
        remove_write_permissions_from_materialized_dir(&visible_tree_root)?;
        Ok(visible_tree_root)
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
