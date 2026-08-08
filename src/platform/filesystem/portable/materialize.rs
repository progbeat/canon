use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> Result<(), String> {
    imp::create_materialized_symlink(target, link).map_err(super::filesystem_error)
}

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    imp::hardlink_file_or_copy_symlink(source, target).map_err(super::filesystem_error)
}
