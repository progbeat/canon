use std::fs;
use std::io;

#[cfg(not(unix))]
mod non_unix;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use non_unix as imp;
#[cfg(unix)]
use unix as imp;

pub(super) fn set_template_artifact_create_mode(options: &mut fs::OpenOptions) {
    imp::set_template_artifact_create_mode(options);
}

pub(super) fn set_template_artifact_file_permissions(file: &fs::File) -> io::Result<()> {
    imp::set_template_artifact_file_permissions(file)
}
