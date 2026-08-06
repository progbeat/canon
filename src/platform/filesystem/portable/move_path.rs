use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    imp::mirror_evaluator_codex_home_file(source, target).map_err(super::filesystem_error)
}

pub(crate) fn move_path(source: &Path, target: &Path) -> Result<(), String> {
    imp::move_path(source, target).map_err(super::filesystem_error)
}
