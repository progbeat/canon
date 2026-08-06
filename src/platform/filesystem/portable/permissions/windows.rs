use super::super::windows::{
    windows_dacl, windows_restore_dacl, windows_restore_parent_dacl,
    windows_set_materialized_readonly_dacl, windows_set_no_access_dacl,
};
use super::MaterializedFileMode;
use std::fs;
use std::path::Path;

pub(in super::super) fn make_hook_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(in super::super) fn set_materialized_file_permissions(
    path: &Path,
    mode: MaterializedFileMode,
) -> Result<(), String> {
    let metadata = path_metadata(path)?;
    match mode {
        MaterializedFileMode::Symlink if metadata.file_type().is_symlink() => Ok(()),
        MaterializedFileMode::Symlink => Err(format!(
            "materialized symlink is not a symlink: {}",
            path.display()
        )),
        MaterializedFileMode::ReadOnly | MaterializedFileMode::Executable => {
            if !metadata.file_type().is_file() {
                return Err(format!(
                    "materialized file is not a regular file: {}",
                    path.display()
                ));
            }
            set_materialized_permissions(path, &metadata)
        }
    }
}

pub(in super::super) fn set_materialized_dir_permissions(path: &Path) -> Result<(), String> {
    let metadata = path_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(format!(
            "materialized directory is not a directory: {}",
            path.display()
        ));
    }
    set_materialized_permissions(path, &metadata)
}

fn path_metadata(path: &Path) -> Result<fs::Metadata, String> {
    fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))
}

fn set_materialized_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    set_readonly(path, metadata, true)?;
    windows_set_materialized_readonly_dacl(path, metadata.file_type().is_dir())
}

pub(in super::super) fn set_private_permissions(path: &Path) -> Result<(), String> {
    let metadata = path_metadata(path)?;
    set_private_permissions_with_metadata(path, &metadata)
}

pub(in super::super) fn set_private_permissions_with_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    windows_restore_parent_dacl(path)?;
    set_readonly(path, metadata, false)
}

fn set_readonly(path: &Path, metadata: &fs::Metadata, readonly: bool) -> Result<(), String> {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions).map_err(|err| {
        format!(
            "failed to update permissions for {}: {}",
            path.display(),
            err
        )
    })
}

#[derive(Clone)]
pub(in super::super) struct SecretDirMode {
    permissions: fs::Permissions,
    dacl: Option<Vec<u8>>,
}

pub(in super::super) fn secret_dir_mode(path: &Path) -> Result<SecretDirMode, String> {
    let metadata = secret_dir_metadata(path)?;
    Ok(SecretDirMode {
        permissions: metadata.permissions(),
        dacl: windows_dacl(path)?,
    })
}

pub(in super::super) fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    secret_dir_metadata(path)?;
    windows_set_no_access_dacl(path)
}

pub(in super::super) fn restore_secret_dir_mode(
    path: &Path,
    mode: &SecretDirMode,
) -> Result<(), String> {
    windows_restore_dacl(path, mode.dacl.as_deref())?;
    fs::set_permissions(path, mode.permissions.clone()).map_err(|err| {
        format!(
            "failed to restore secret dir permissions {}: {}",
            path.display(),
            err
        )
    })
}

fn secret_dir_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to stat secret dir {}: {}", path.display(), err))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("secret dir {} is not a directory", path.display()));
    }
    Ok(metadata)
}
