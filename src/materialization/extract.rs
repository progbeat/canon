use super::{
    entry_path::relative_path_from_git_path,
    permissions::remove_write_permissions_from_extracted_file, visible_tree::VisibleTree,
    TreeMaterializer,
};
use crate::git::{GitBlobReader, TrackedFile};
use crate::platform::filesystem;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

impl TreeMaterializer {
    pub(super) fn unpack_missing_visible_entries(
        &self,
        visible_tree: &VisibleTree,
    ) -> Result<(), String> {
        let missing = {
            let unpacked = self.unpacked_paths.borrow();
            visible_tree
                .entry_paths
                .iter()
                .filter(|file| !unpacked.contains(&file.path))
                .cloned()
                .collect::<Vec<_>>()
        };
        if missing.is_empty() {
            return Ok(());
        }

        let object_ids = missing
            .iter()
            .filter(|file| file.is_blob_file_entry())
            .map(|file| file.object_id.clone())
            .collect::<Vec<_>>();
        let blobs = self.read_missing_blobs(&object_ids)?;
        let mut blobs = blobs.into_iter();
        for file in missing {
            let content = if file.is_blob_file_entry() {
                blobs
                    .next()
                    .expect("blob content count matched blob-backed visible entries")
            } else {
                materialized_non_blob_content(&file)?
            };
            extract_visible_entry(self, &file, &content)?;
            self.unpacked_paths.borrow_mut().insert(file.path);
        }
        Ok(())
    }

    pub(super) fn read_missing_blobs(&self, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
        let mut reader = self.blob_reader.borrow_mut();
        if reader.is_none() {
            *reader = Some(GitBlobReader::new(&self.source_root)?);
        }
        reader
            .as_mut()
            .expect("git blob reader was initialized")
            .read_blobs(object_ids)
    }
}

fn extract_visible_entry(
    tree_materializer: &TreeMaterializer,
    file: &TrackedFile,
    content: &[u8],
) -> Result<(), String> {
    let relative = relative_path_from_git_path(&file.path)?;
    let target = tree_materializer
        .materialized_input
        .extracted_files_dir
        .join(&relative);
    if let Some(parent) = target.parent() {
        filesystem::create_private_dir_all(parent).map_err(|err| {
            format!(
                "failed to create evaluator input directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }
    tree_materializer
        .materialized_input
        .prepare_extracted_path_for_write(&target)?;
    if file.mode == "120000" {
        filesystem::create_materialized_symlink(content, &target)?;
    } else {
        fs::write(&target, content).map_err(|err| {
            format!(
                "failed to write evaluator input file {}: {}",
                target.display(),
                err
            )
        })?;
    }
    remove_write_permissions_from_extracted_file(&target, &file.mode)
}

impl super::MaterializedProjectInput {
    fn prepare_extracted_path_for_write(&self, target: &Path) -> Result<(), String> {
        let Some(journal) = &self.rollback_journal else {
            return remove_existing_extracted_path(target);
        };
        let backup = match fs::symlink_metadata(target) {
            Ok(_) => Some(move_to_unique_rollback_backup(target)?),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => {
                return Err(format!(
                    "failed to inspect evaluator input file {}: {}",
                    target.display(),
                    err
                ));
            }
        };
        journal
            .borrow_mut()
            .extracted_path_changes
            .push(super::RollbackExtractedPathChange {
                target: target.to_path_buf(),
                backup,
            });
        Ok(())
    }
}

fn move_to_unique_rollback_backup(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("evaluator input path has no parent: {}", target.display()))?;
    for _ in 0..64 {
        let random =
            getrandom::u64().map_err(|err| format!("failed to choose input backup path: {err}"))?;
        let backup = parent.join(format!(
            ".canon-rollback-backup-{}-{random:016x}",
            std::process::id()
        ));
        match fs::symlink_metadata(&backup) {
            Ok(_) => continue,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "failed to inspect evaluator input backup {}: {}",
                    backup.display(),
                    err
                ));
            }
        }
        fs::rename(target, &backup).map_err(|err| {
            format!(
                "failed to preserve evaluator input path {} at {}: {}",
                target.display(),
                backup.display(),
                err
            )
        })?;
        return Ok(backup);
    }
    Err(format!(
        "failed to allocate a unique evaluator input backup beside {}",
        target.display()
    ))
}

fn remove_existing_extracted_path(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::remove_dir_all(target).map_err(|err| {
                format!(
                    "failed to replace evaluator input directory {}: {}",
                    target.display(),
                    err
                )
            })
        }
        Ok(_) => fs::remove_file(target).map_err(|err| {
            format!(
                "failed to replace evaluator input file {}: {}",
                target.display(),
                err
            )
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to inspect evaluator input file {}: {}",
            target.display(),
            err
        )),
    }
}

fn materialized_non_blob_content(file: &TrackedFile) -> Result<Vec<u8>, String> {
    if file.mode == "160000" {
        return Ok(format!("gitlink {}\n", file.object_id).into_bytes());
    }
    Err(format!(
        "unsupported visible tree entry mode {} for {}",
        file.mode,
        String::from_utf8_lossy(&file.path)
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::test_support::{empty_test_agent, git_project};
    use super::*;
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::hash::full_scope;
    use std::os::unix::fs::symlink;
    use std::process::Command;

    #[test] // xpec: 1t
    fn materialized_symlink_does_not_follow_its_missing_target() {
        let root = git_project("missing-symlink-target");
        symlink("missing-target", root.join("link.txt")).unwrap();
        let output = Command::new("git")
            .args(["add", "link.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let scope = full_scope();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &empty_test_agent(), &scope)
            .unwrap();
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();

        let scope_root = tree_materializer
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .unwrap();

        let extracted = scope_root.join("link.txt");
        assert!(fs::symlink_metadata(&extracted)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(extracted).unwrap(),
            Path::new("missing-target")
        );
        let _ = fs::remove_dir_all(root);
    }
}
