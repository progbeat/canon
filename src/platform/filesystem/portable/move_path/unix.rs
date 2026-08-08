use super::super::directory_tree::unix::{
    make_directory_tree_private_with_restore, DirectoryTreeModeGuard,
};
use super::super::permissions::unix::{
    directory_permissions, fchmod_open_path, open_source_directory_for_move,
};
use super::super::unix::{PlatformError, PlatformResult};
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(super) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> PlatformResult<()> {
    symlink(source, target).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to symlink evaluator CODEX_HOME file {} to {}",
                target.display(),
                source.display()
            ),
            err,
        )
    })
}

pub(super) fn move_path(source: &Path, target: &Path) -> PlatformResult<()> {
    // This function is the naive isolation policy's abstract `move` operation.
    // `move` describes the completed filesystem state (source absent, target
    // present), not one particular syscall. `rename(2)` implements it within a
    // filesystem; a composite operation is required when EXDEV says that
    // filesystem move cannot cross the device boundary.
    let result = move_path_preserving_directory_permissions(source, target);
    if result.is_ok() {
        // xpec: Hj
        assert!(
            matches!(
                fs::symlink_metadata(source),
                Err(err) if err.kind() == io::ErrorKind::NotFound
            ),
            "successful filesystem move must remove its source entry"
        );
        // xpec: Hj
        assert!(
            fs::symlink_metadata(target).is_ok(),
            "successful filesystem move must create its target entry"
        );
    }
    result
}

fn move_path_preserving_directory_permissions(source: &Path, target: &Path) -> PlatformResult<()> {
    let Some(directory) = open_source_directory_for_move(source)? else {
        return rename_path(source, target).map_err(|err| move_path_error(source, target, err));
    };
    let mode = directory_permissions(source, &directory)?.mode();
    // Some supported filesystems reject moving canon's read-only materialized
    // directories. This temporary mode is part of the move implementation and
    // is restored at the destination before the operation returns.
    fchmod_open_path(source, &directory, 0o700)?;
    match rename_path(source, target) {
        Ok(()) => {}
        Err(rename_err) if rename_err.raw_os_error() == Some(libc::EXDEV) => {
            // Do not return between the copy and removal steps: callers observe
            // one completed move operation or an error with rollback attempted.
            if let Err(copy_err) = complete_cross_filesystem_directory_move(source, target, mode) {
                return Err(restore_source_directory_permissions_after_failed_move(
                    source, &directory, mode, copy_err,
                ));
            }
            return Ok(());
        }
        Err(rename_err) => {
            return Err(restore_source_directory_permissions_after_failed_move(
                source,
                &directory,
                mode,
                move_path_error(source, target, rename_err),
            ));
        }
    }
    if let Err(restore_err) = fchmod_open_path(target, &directory, mode) {
        return Err(rollback_moved_directory_after_restore_failure(
            source,
            target,
            &directory,
            mode,
            restore_err,
        ));
    }
    Ok(())
}

fn restore_source_directory_permissions_after_failed_move(
    source: &Path,
    directory: &fs::File,
    mode: u32,
    rename_err: PlatformError,
) -> PlatformError {
    match fchmod_open_path(source, directory, mode) {
        Ok(()) => rename_err,
        Err(restore_err) => PlatformError::chain(vec![rename_err, restore_err]),
    }
}

fn rollback_moved_directory_after_restore_failure(
    source: &Path,
    target: &Path,
    directory: &fs::File,
    mode: u32,
    restore_err: PlatformError,
) -> PlatformError {
    let mut errors = vec![restore_err];
    match rename_path(target, source) {
        Ok(()) => {
            if let Err(source_restore_err) = fchmod_open_path(source, directory, mode) {
                errors.push(source_restore_err);
            }
        }
        Err(rollback_err) => errors.push(PlatformError::with_source(
            "failed to roll back moved directory",
            move_path_error(target, source, rollback_err),
        )),
    }
    PlatformError::chain(errors)
}

fn complete_cross_filesystem_directory_move(
    source: &Path,
    target: &Path,
    mode: u32,
) -> PlatformResult<()> {
    if let Err(copy_err) = copy_directory(source, target) {
        let _ = remove_directory_tree(target);
        return Err(copy_err);
    }
    // xpec: Hj
    // Finish every fallible target-side step while the source is still intact.
    // After source removal succeeds, the cross-device move is complete.
    if let Err(permission_err) = fs::set_permissions(target, fs::Permissions::from_mode(mode))
        .map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to set moved directory permissions {}",
                    target.display()
                ),
                err,
            )
        })
    {
        let rollback_err = remove_directory_tree(target).err();
        return Err(match rollback_err {
            Some(rollback_err) => PlatformError::chain(vec![permission_err, rollback_err]),
            None => permission_err,
        });
    }
    if let Err(remove_err) = remove_directory_tree(source) {
        let rollback_err = remove_directory_tree(target).err();
        let remove_err = PlatformError::with_source(
            format!(
                "failed to remove isolated source path {} after copy to {}",
                source.display(),
                target.display()
            ),
            remove_err,
        );
        return Err(match rollback_err {
            Some(rollback_err) => PlatformError::chain(vec![remove_err, rollback_err]),
            None => remove_err,
        });
    }
    Ok(())
}

fn rename_path(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

fn move_path_error(source: &Path, target: &Path, err: io::Error) -> PlatformError {
    PlatformError::io(
        format!(
            "failed to move isolated path {} to {}",
            source.display(),
            target.display()
        ),
        err,
    )
}

fn copy_directory(source: &Path, target: &Path) -> PlatformResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        PlatformError::io(
            format!("failed to inspect directory {}", source.display()),
            err,
        )
    })?;
    copy_directory_with_metadata(source, target, metadata)
}

fn copy_directory_with_metadata(
    source: &Path,
    target: &Path,
    metadata: fs::Metadata,
) -> PlatformResult<()> {
    if !metadata.file_type().is_dir() {
        return Err(PlatformError::message(format!(
            "refusing to copy non-directory {}",
            source.display()
        )));
    }
    fs::create_dir(target).map_err(|err| {
        PlatformError::io(
            format!("failed to create copied directory {}", target.display()),
            err,
        )
    })?;
    for entry in fs::read_dir(source).map_err(|err| {
        PlatformError::io(
            format!("failed to read directory {}", source.display()),
            err,
        )
    })? {
        let entry = entry.map_err(|err| {
            PlatformError::io(
                format!("failed to read directory {}", source.display()),
                err,
            )
        })?;
        copy_path(&entry.path(), &target.join(entry.file_name()))?;
    }
    fs::set_permissions(target, metadata.permissions()).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to set copied directory permissions {}",
                target.display()
            ),
            err,
        )
    })
}

fn copy_path(source: &Path, target: &Path) -> PlatformResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        PlatformError::io(format!("failed to inspect path {}", source.display()), err)
    })?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        return copy_directory_with_metadata(source, target, metadata);
    }
    if file_type.is_symlink() {
        let link_target = fs::read_link(source).map_err(|err| {
            PlatformError::io(format!("failed to read symlink {}", source.display()), err)
        })?;
        return symlink(&link_target, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to copy symlink {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        });
    }
    if file_type.is_file() {
        fs::copy(source, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to copy file {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        })?;
        return fs::set_permissions(target, metadata.permissions()).map_err(|err| {
            PlatformError::io(
                format!("failed to set copied file permissions {}", target.display()),
                err,
            )
        });
    }
    Err(PlatformError::message(format!(
        "refusing to copy unsupported path type {}",
        source.display()
    )))
}

fn remove_directory_tree(path: &Path) -> PlatformResult<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            let restored_modes = make_directory_tree_removable(path)?;
            match fs::remove_dir_all(path) {
                Ok(()) => Ok(()),
                Err(err) => {
                    let remove_err = PlatformError::io(
                        format!("failed to remove directory {}", path.display()),
                        err,
                    );
                    match restored_modes.restore() {
                        Ok(()) => Err(remove_err),
                        Err(restore_err) => {
                            Err(PlatformError::chain(vec![remove_err, restore_err]))
                        }
                    }
                }
            }
        }
        Err(err) => Err(PlatformError::io(
            format!("failed to remove directory {}", path.display()),
            err,
        )),
    }
}

fn make_directory_tree_removable(path: &Path) -> PlatformResult<DirectoryTreeModeGuard> {
    make_directory_tree_private_with_restore(path)
}

#[cfg(test)]
mod tests {
    use super::{complete_cross_filesystem_directory_move, make_directory_tree_removable};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: Hj
    fn directory_mode_restorer_restores_nested_modes_after_failed_retry() {
        let root = temp_root("mode-restore");
        let nested = root.join("source").join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::set_permissions(root.join("source"), fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o300)).unwrap();

        let restorer = make_directory_tree_removable(&root.join("source")).unwrap();
        assert_eq!(mode(root.join("source")) & 0o777, 0o700);
        assert_eq!(mode(&nested) & 0o777, 0o700);

        restorer.restore().unwrap();
        assert_eq!(mode(root.join("source")) & 0o777, 0o500);
        assert_eq!(mode(&nested) & 0o777, 0o300);

        fs::set_permissions(root.join("source"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: Hj
    fn cross_filesystem_fallback_completes_the_move_postcondition() {
        let root = temp_root("cross-filesystem-postcondition");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "content").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o500)).unwrap();

        complete_cross_filesystem_directory_move(&source, &target, 0o500).unwrap();

        assert!(fs::symlink_metadata(&source)
            .is_err_and(|err| { err.kind() == std::io::ErrorKind::NotFound }));
        assert_eq!(fs::read_to_string(target.join("file")).unwrap(), "content");
        assert_eq!(mode(&target) & 0o777, 0o500);

        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn mode(path: impl AsRef<std::path::Path>) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode()
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("canon-unix-move-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
