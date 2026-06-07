use super::security::{
    windows_dacl, windows_restore_dacl, windows_restore_parent_dacl,
    windows_set_materialized_readonly_dacl, windows_set_no_access_dacl,
};
use std::fs;
use std::path::Path;

pub(crate) fn make_hook_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_materialized_permissions(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    set_readonly(path, &metadata, true)?;
    windows_set_materialized_readonly_dacl(path, metadata.file_type().is_dir())
}

pub(crate) fn set_private_permissions(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    windows_restore_parent_dacl(path)?;
    set_readonly(path, &metadata, false)
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
pub(crate) struct SecretDirMode {
    permissions: fs::Permissions,
    dacl: Option<Vec<u8>>,
}

pub(crate) fn secret_dir_mode(path: &Path) -> Result<SecretDirMode, String> {
    let metadata = secret_dir_metadata(path)?;
    Ok(SecretDirMode {
        permissions: metadata.permissions(),
        dacl: windows_dacl(path)?,
    })
}

pub(crate) fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    secret_dir_metadata(path)?;
    windows_set_no_access_dacl(path)
}

pub(crate) fn restore_secret_dir_mode(path: &Path, mode: &SecretDirMode) -> Result<(), String> {
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
