use super::chmod::{directory_permissions, fchmod_open_path, open_source_directory_for_move};
use super::error::{PlatformError, PlatformResult};
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> PlatformResult<()> {
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

pub(crate) fn move_path(source: &Path, target: &Path) -> PlatformResult<()> {
    move_path_preserving_directory_permissions(source, target)
}

fn move_path_preserving_directory_permissions(source: &Path, target: &Path) -> PlatformResult<()> {
    let Some(directory) = open_source_directory_for_move(source)? else {
        return rename_path(source, target).map_err(|err| move_path_error(source, target, err));
    };
    let mode = directory_permissions(source, &directory)?.mode();
    fchmod_open_path(source, &directory, 0o700)?;
    match rename_path(source, target) {
        Ok(()) => {}
        Err(rename_err) if rename_err.raw_os_error() == Some(libc::EXDEV) => {
            if let Err(copy_err) = move_directory_across_devices(source, target, mode) {
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

fn move_directory_across_devices(source: &Path, target: &Path, mode: u32) -> PlatformResult<()> {
    if let Err(copy_err) = copy_directory(source, target) {
        let _ = remove_directory_tree(target);
        return Err(copy_err);
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
    fs::set_permissions(target, fs::Permissions::from_mode(mode)).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to set moved directory permissions {}",
                target.display()
            ),
            err,
        )
    })?;
    Ok(())
}

fn remove_directory_tree(path: &Path) -> PlatformResult<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            make_directory_tree_removable(path)?;
            fs::remove_dir_all(path).map_err(|err| {
                PlatformError::io(
                    format!("failed to remove directory {}", path.display()),
                    err,
                )
            })
        }
        Err(err) => Err(PlatformError::io(
            format!("failed to remove directory {}", path.display()),
            err,
        )),
    }
}

fn make_directory_tree_removable(path: &Path) -> PlatformResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PlatformError::io(
                format!("failed to inspect directory {}", path.display()),
                err,
            ));
        }
    };
    if !metadata.file_type().is_dir() {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|err| {
        PlatformError::io(
            format!("failed to make directory removable {}", path.display()),
            err,
        )
    })?;
    for entry in fs::read_dir(path).map_err(|err| {
        PlatformError::io(format!("failed to read directory {}", path.display()), err)
    })? {
        let entry = entry.map_err(|err| {
            PlatformError::io(format!("failed to read directory {}", path.display()), err)
        })?;
        make_directory_tree_removable(&entry.path())?;
    }
    Ok(())
}

fn copy_directory(source: &Path, target: &Path) -> PlatformResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        PlatformError::io(
            format!("failed to inspect directory {}", source.display()),
            err,
        )
    })?;
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
        return copy_directory(source, target);
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
