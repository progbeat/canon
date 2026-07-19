use super::path::relative_path_from_git_path;
use super::permissions::remove_write_permissions_from_extracted_file;
use super::visible_tree::VisibleTree;
use super::StagedWorktreeView;
use crate::git::{GitBlobReader, StagedTrackedFile};
use crate::platform;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

impl StagedWorktreeView {
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
    staged_view: &StagedWorktreeView,
    file: &StagedTrackedFile,
    content: &[u8],
) -> Result<(), String> {
    let relative = relative_path_from_git_path(&file.path)?;
    let target = staged_view.lazy_tree_dir.join(&relative);
    if let Some(parent) = target.parent() {
        platform::create_private_dir_all(parent).map_err(|err| {
            format!(
                "failed to create evaluator lazy directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }
    staged_view.prepare_lazy_path_for_write(&target)?;
    if file.mode == "120000" {
        platform::create_materialized_symlink(content, &target)?;
    } else {
        fs::write(&target, content).map_err(|err| {
            format!(
                "failed to write evaluator lazy file {}: {}",
                target.display(),
                err
            )
        })?;
    }
    remove_write_permissions_from_extracted_file(&target, &file.mode)
}

impl StagedWorktreeView {
    fn prepare_lazy_path_for_write(&self, target: &Path) -> Result<(), String> {
        let Some(journal) = &self.invocation_journal else {
            return remove_existing_lazy_path(target);
        };
        let backup = match fs::symlink_metadata(target) {
            Ok(_) => Some(move_to_unique_invocation_backup(target)?),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => {
                return Err(format!(
                    "failed to inspect evaluator lazy file {}: {}",
                    target.display(),
                    err
                ));
            }
        };
        journal
            .borrow_mut()
            .lazy_path_changes
            .push(super::InvocationLazyPathChange {
                target: target.to_path_buf(),
                backup,
            });
        Ok(())
    }
}

fn move_to_unique_invocation_backup(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("evaluator lazy path has no parent: {}", target.display()))?;
    for _ in 0..64 {
        let random =
            getrandom::u64().map_err(|err| format!("failed to choose lazy backup path: {err}"))?;
        let backup = parent.join(format!(
            ".canon-invocation-backup-{}-{random:016x}",
            std::process::id()
        ));
        match fs::symlink_metadata(&backup) {
            Ok(_) => continue,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "failed to inspect evaluator lazy backup {}: {}",
                    backup.display(),
                    err
                ));
            }
        }
        fs::rename(target, &backup).map_err(|err| {
            format!(
                "failed to preserve evaluator lazy path {} at {}: {}",
                target.display(),
                backup.display(),
                err
            )
        })?;
        return Ok(backup);
    }
    Err(format!(
        "failed to allocate a unique evaluator lazy backup beside {}",
        target.display()
    ))
}

fn remove_existing_lazy_path(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::remove_dir_all(target).map_err(|err| {
                format!(
                    "failed to replace evaluator lazy directory {}: {}",
                    target.display(),
                    err
                )
            })
        }
        Ok(_) => fs::remove_file(target).map_err(|err| {
            format!(
                "failed to replace evaluator lazy file {}: {}",
                target.display(),
                err
            )
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to inspect evaluator lazy file {}: {}",
            target.display(),
            err
        )),
    }
}

fn materialized_non_blob_content(file: &StagedTrackedFile) -> Result<Vec<u8>, String> {
    if file.mode == "160000" {
        return Ok(format!("gitlink {}\n", file.object_id).into_bytes());
    }
    Err(format!(
        "unsupported visible tree entry mode {} for {}",
        file.mode,
        String::from_utf8_lossy(&file.path)
    ))
}
