use super::chmod::{directory_permissions, fchmod_open_path, open_source_directory_for_move};
use super::error::{PlatformError, PlatformResult};
use std::fs;
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
        return rename_path(source, target);
    };
    let mode = directory_permissions(source, &directory)?.mode();
    fchmod_open_path(source, &directory, 0o700)?;
    if let Err(rename_err) = rename_path(source, target) {
        return Err(restore_source_directory_permissions_after_failed_move(
            source, &directory, mode, rename_err,
        ));
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
            rollback_err,
        )),
    }
    PlatformError::chain(errors)
}

fn rename_path(source: &Path, target: &Path) -> PlatformResult<()> {
    fs::rename(source, target).map_err(|err| {
        PlatformError::io(
            format!(
                "failed to move isolated path {} to {}",
                source.display(),
                target.display()
            ),
            err,
        )
    })
}
