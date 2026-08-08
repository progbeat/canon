use super::filesystem_error;
use std::path::Path;

#[cfg(unix)]
pub(super) mod unix;
#[cfg(windows)]
pub(super) mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn make_hook_executable(path: &Path) -> Result<(), String> {
    imp::make_hook_executable(path).map_err(filesystem_error)
}

#[derive(Clone, Copy)]
pub(super) enum MaterializedFileMode {
    ReadOnly,
    Executable,
    Symlink,
}

impl MaterializedFileMode {
    fn from_git_mode(path: &Path, mode: &str) -> Result<MaterializedFileMode, String> {
        match mode {
            "100644" | "160000" => Ok(MaterializedFileMode::ReadOnly),
            "100755" => Ok(MaterializedFileMode::Executable),
            "120000" => Ok(MaterializedFileMode::Symlink),
            _ => Err(format!(
                "unsupported materialized file mode {} for {}",
                mode,
                path.display()
            )),
        }
    }
}

pub(crate) fn set_materialized_file_permissions(path: &Path, mode: &str) -> Result<(), String> {
    let mode = MaterializedFileMode::from_git_mode(path, mode)?;
    imp::set_materialized_file_permissions(path, mode).map_err(filesystem_error)
}

pub(crate) fn set_materialized_dir_permissions(path: &Path) -> Result<(), String> {
    imp::set_materialized_dir_permissions(path).map_err(filesystem_error)
}

#[derive(Clone)]
pub(crate) struct SecretDirMode {
    inner: imp::SecretDirMode,
}

pub(crate) fn secret_dir_mode(path: &Path) -> Result<SecretDirMode, String> {
    imp::secret_dir_mode(path)
        .map(|inner| SecretDirMode { inner })
        .map_err(filesystem_error)
}

pub(crate) fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    imp::chmod_secret_dir_no_access(path).map_err(filesystem_error)
}

pub(crate) fn restore_secret_dir_mode(path: &Path, mode: &SecretDirMode) -> Result<(), String> {
    imp::restore_secret_dir_mode(path, &mode.inner).map_err(filesystem_error)
}
