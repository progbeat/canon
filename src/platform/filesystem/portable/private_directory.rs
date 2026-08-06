use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    imp::create_private_dir(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    imp::create_private_dir_all(path)
}
