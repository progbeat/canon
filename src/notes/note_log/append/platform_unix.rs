use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(super) fn open_append_target(path: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(super) fn open_rollback_target(path: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(super) fn rollback_needs_reopen(_err: &std::io::Error) -> bool {
    false
}
