use std::fs;
use std::io;

pub(super) fn set_template_artifact_create_mode(_options: &mut fs::OpenOptions) {}

pub(super) fn set_template_artifact_file_permissions(_file: &fs::File) -> io::Result<()> {
    Ok(())
}
