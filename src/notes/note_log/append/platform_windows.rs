use std::fs;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

pub(super) fn open_append_target(path: &Path) -> Result<fs::File, String> {
    reject_append_symlink(path)?;
    let file = fs::OpenOptions::new()
        .append(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let metadata = file
        .metadata()
        .map_err(|err| format!("failed to inspect opened {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to open symlink {}", path.display()));
    }
    if !metadata.file_type().is_file() {
        return Err(format!("refusing to open non-file {}", path.display()));
    }
    Ok(file)
}

pub(super) fn open_rollback_target(path: &Path) -> Result<fs::File, String> {
    reject_append_symlink(path)?;
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(super) fn rollback_needs_reopen(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied
}

fn reject_append_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to open symlink {}", path.display()));
    }
    Ok(())
}
