use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(super) fn set_template_artifact_create_mode(options: &mut fs::OpenOptions) {
    options.mode(0o600);
}

pub(super) fn set_template_artifact_file_permissions(file: &fs::File) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(0o600))
}
