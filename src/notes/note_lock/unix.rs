use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

pub(super) struct NoteLock {
    _file: fs::File,
}

pub(super) fn lock_note_sidecar(path: &Path, _stale_after: Duration) -> Result<NoteLock, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|err| format!("failed to open lock {}: {}", path.display(), err))?;
    lock_note_file(&file, path)?;
    Ok(NoteLock { _file: file })
}

fn lock_note_file(file: &fs::File, path: &Path) -> Result<(), String> {
    loop {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(format!("failed to lock {}: {}", path.display(), err));
    }
}
