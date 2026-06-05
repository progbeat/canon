use std::fs;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const NOTE_LOCK_HEARTBEAT_SECS: u64 = 60;
const NOTE_LOCK_HEARTBEAT_POLL: Duration = Duration::from_millis(25);
const NOTE_LOCK_RETRY_COUNT: usize = 1000;
const NOTE_LOCK_RETRY_SLEEP: Duration = Duration::from_millis(10);

pub(super) struct NoteLock {
    file: Option<fs::File>,
    path: PathBuf,
    token: String,
    stop_heartbeat: Arc<AtomicBool>,
    heartbeat: Option<JoinHandle<()>>,
}

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

enum NoteLockState {
    Missing,
    Held,
    Stale,
}

pub(super) fn lock_note_sidecar(path: &Path, stale_after: Duration) -> Result<NoteLock, String> {
    for _ in 0..NOTE_LOCK_RETRY_COUNT {
        match create_note_lock(path) {
            Ok(file) => return new_note_lock(path, file),
            Err(err) if note_lock_create_error_is_retryable(&err) => {
                if matches!(note_lock_state(path, stale_after)?, NoteLockState::Stale)
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

fn create_note_lock(path: &Path) -> Result<fs::File, io::Error> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

fn note_lock_create_error_is_retryable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
    )
}

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

fn note_lock_token() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("pid={} token={}", process::id(), timestamp)
}

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

fn write_note_lock_owner(file: &mut fs::File, path: &Path, token: &str) -> Result<(), String> {
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| writeln!(file, "{}", token))
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_data())
        .map_err(|err| format!("failed to refresh lock {}: {}", path.display(), err))
}

fn note_lock_state(path: &Path, stale_after: Duration) -> Result<NoteLockState, String> {
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
    Ok(if age >= stale_after {
        NoteLockState::Stale
    } else {
        NoteLockState::Held
    })
}

fn remove_stale_lock_for_retry(path: &Path) -> Result<bool, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(err) => Err(format!(
            "failed to remove stale lock {}: {}",
            path.display(),
            err
        )),
    }
}

fn remove_note_lock_if_owned(path: &Path, token: &str) -> Result<(), String> {
    match fs::read_to_string(path) {
        Ok(content) if content.lines().next() == Some(token) => remove_note_lock_file(path),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to inspect lock {}: {}",
            path.display(),
            err
        )),
    }
}

fn remove_note_lock_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to remove stale lock {}: {}",
            path.display(),
            err
        )),
    }
}
