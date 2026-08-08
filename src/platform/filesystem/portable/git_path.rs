use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
pub(super) mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> Result<PathBuf, String> {
    imp::remove_git_stdout_record_terminator(&mut bytes)?;
    #[cfg(unix)]
    {
        Ok(imp::path_from_git_bytes(bytes))
    }
    #[cfg(windows)]
    {
        imp::path_from_git_bytes(bytes)
    }
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        Ok(imp::git_path_bytes(path))
    }
    #[cfg(windows)]
    {
        imp::git_path_bytes(path)
    }
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    #[cfg(unix)]
    {
        Ok(imp::os_string_from_bytes(bytes))
    }
    #[cfg(windows)]
    {
        imp::os_string_from_bytes(bytes)
    }
}
