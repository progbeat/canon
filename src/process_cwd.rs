use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static PROCESS_CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn with_current_dir<T, F>(dir: &Path, f: F) -> Result<T, String>
where
    F: FnOnce() -> T,
{
    // Process cwd is global, so temporary cwd changes live behind one shared
    // helper instead of component-private locks.
    let _lock = PROCESS_CWD_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "process cwd lock was poisoned".to_string())?;
    let previous = env::current_dir().map_err(|err| {
        format!(
            "failed to read current dir before entering {}: {}",
            dir.display(),
            err
        )
    })?;
    env::set_current_dir(dir)
        .map_err(|err| format!("failed to enter current dir {}: {}", dir.display(), err))?;
    let restore = RestoreCurrentDir {
        previous,
        restored: false,
    };
    let result = f();
    restore.restore()?;
    Ok(result)
}

struct RestoreCurrentDir {
    previous: PathBuf,
    restored: bool,
}

impl RestoreCurrentDir {
    fn restore(mut self) -> Result<(), String> {
        env::set_current_dir(&self.previous).map_err(|err| {
            format!(
                "failed to restore current dir {}: {}",
                self.previous.display(),
                err
            )
        })?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for RestoreCurrentDir {
    fn drop(&mut self) {
        if !self.restored {
            let _ = env::set_current_dir(&self.previous);
        }
    }
}
