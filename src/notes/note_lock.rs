#[cfg(not(unix))]
use crate::notes::lock::{
    create_lock_file, remove_stale_lock, remove_stale_lock_for_retry, stale_lock_age,
};
use crate::project_types::Note;
use std::fs;
use std::io;
#[cfg(not(unix))]
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;
#[cfg(not(unix))]
use std::process;
#[cfg(not(unix))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(unix))]
use std::sync::Arc;
#[cfg(not(unix))]
use std::thread::{self, JoinHandle};
#[cfg(not(unix))]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(unix))]
const NOTE_LOCK_HEARTBEAT_SECS: u64 = 60;
#[cfg(not(unix))]
const NOTE_LOCK_HEARTBEAT_POLL: Duration = Duration::from_millis(25);
#[cfg(not(unix))]
const NOTE_LOCK_RETRY_COUNT: usize = 1000;
#[cfg(not(unix))]
const NOTE_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(10);

// Note compaction replaces the note file, so same-note mutations use a sidecar
// lock that stays stable across append-log writes, compaction, and delete.
#[cfg(unix)]
pub(crate) struct NoteLock {
    _file: fs::File,
}

#[cfg(not(unix))]
pub(crate) struct NoteLock {
    file: Option<fs::File>,
    path: PathBuf,
    token: String,
    stop_heartbeat: Arc<AtomicBool>,
    heartbeat: Option<JoinHandle<()>>,
}

#[cfg(not(unix))]
impl Drop for NoteLock {
    fn drop(&mut self) {
        self.stop_heartbeat.store(true, Ordering::Release);
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        drop(self.file.take());
        let _ = remove_note_lock_if_owned(&self.path, &self.token);
    }
}

#[cfg(not(unix))]
enum NoteLockState {
    Missing,
    Held,
    Stale,
}

pub(crate) fn lock_note(note: &Note) -> Result<NoteLock, String> {
    let path = note.path.with_extension("md.lock");
    lock_note_at_path(&path)
}

#[cfg(unix)]
fn lock_note_at_path(path: &Path) -> Result<NoteLock, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|err| format!("failed to open lock {}: {}", path.display(), err))?;
    lock_note_file(&file, path)?;
    Ok(NoteLock { _file: file })
}

#[cfg(not(unix))]
fn lock_note_at_path(path: &Path) -> Result<NoteLock, String> {
    for _ in 0..NOTE_LOCK_RETRY_COUNT {
        match create_note_lock(path) {
            Ok(file) => return new_note_lock(path, file),
            Err(err) if note_lock_create_error_is_retryable(&err) => {
                if matches!(note_lock_state(path)?, NoteLockState::Stale)
                    && remove_stale_lock_for_retry(path)?
                {
                    continue;
                }
                thread::sleep(NOTE_LOCK_RETRY_SLEEP);
            }
            Err(err) => return Err(format!("failed to lock {}: {}", path.display(), err)),
        }
    }
    Err(format!(
        "failed to lock {}: lock is already held",
        path.display()
    ))
}

#[cfg(not(unix))]
fn create_note_lock(path: &Path) -> Result<fs::File, io::Error> {
    create_lock_file(path)
}

#[cfg(not(unix))]
fn note_lock_create_error_is_retryable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
    )
}

#[cfg(not(unix))]
fn new_note_lock(path: &Path, mut file: fs::File) -> Result<NoteLock, String> {
    let token = note_lock_token();
    write_note_lock_owner(&mut file, path, &token)?;
    let heartbeat_file = file
        .try_clone()
        .map_err(|err| format!("failed to clone lock {}: {}", path.display(), err))?;
    let stop_heartbeat = Arc::new(AtomicBool::new(false));
    let heartbeat = start_note_lock_heartbeat(
        path.to_path_buf(),
        token.clone(),
        heartbeat_file,
        Arc::clone(&stop_heartbeat),
    );
    Ok(NoteLock {
        file: Some(file),
        path: path.to_path_buf(),
        token,
        stop_heartbeat,
        heartbeat: Some(heartbeat),
    })
}

#[cfg(not(unix))]
fn note_lock_token() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("pid={} token={}", process::id(), timestamp)
}

#[cfg(not(unix))]
fn start_note_lock_heartbeat(
    path: PathBuf,
    token: String,
    mut file: fs::File,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || loop {
        let mut slept = Duration::ZERO;
        let interval = Duration::from_secs(NOTE_LOCK_HEARTBEAT_SECS);
        while slept < interval {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let remaining = interval.saturating_sub(slept);
            let step = remaining.min(NOTE_LOCK_HEARTBEAT_POLL);
            thread::sleep(step);
            slept += step;
        }
        if stop.load(Ordering::Acquire) {
            return;
        }
        let _ = write_note_lock_owner(&mut file, &path, &token);
    })
}

#[cfg(not(unix))]
fn write_note_lock_owner(file: &mut fs::File, path: &Path, token: &str) -> Result<(), String> {
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| writeln!(file, "{}", token))
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_data())
        .map_err(|err| format!("failed to refresh lock {}: {}", path.display(), err))
}

#[cfg(not(unix))]
fn note_lock_state(path: &Path) -> Result<NoteLockState, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("refusing to use symlink {}", path.display()));
        }
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(NoteLockState::Missing),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return Ok(NoteLockState::Held);
        }
        Err(err) => return Err(format!("failed to inspect {}: {}", path.display(), err)),
    };
    let modified = match metadata.modified() {
        Ok(modified) => modified,
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return Ok(NoteLockState::Held);
        }
        Err(err) => {
            return Err(format!(
                "failed to inspect mtime for {}: {}",
                path.display(),
                err
            ));
        }
    };
    let age = match modified.elapsed() {
        Ok(age) => age,
        Err(err) => {
            return Err(format!(
                "failed to inspect age for {}: {}",
                path.display(),
                err
            ));
        }
    };
    Ok(if stale_lock_age(age) {
        NoteLockState::Stale
    } else {
        NoteLockState::Held
    })
}

#[cfg(not(unix))]
fn remove_note_lock_if_owned(path: &Path, token: &str) -> Result<(), String> {
    match fs::read_to_string(path) {
        Ok(content) if content.lines().next() == Some(token) => remove_stale_lock(path),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to inspect lock {}: {}",
            path.display(),
            err
        )),
    }
}

#[cfg(unix)]
fn lock_note_file(file: &fs::File, path: &Path) -> Result<(), String> {
    use std::os::fd::AsRawFd;
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
