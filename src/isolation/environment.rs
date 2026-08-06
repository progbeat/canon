use crate::platform::filesystem;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const CANON_SECRET_DIR: &str = "CANON_SECRET_DIR";
pub(super) const CANON_SANDBOX_DIR: &str = "CANON_SANDBOX_DIR";

pub(crate) fn prepare_evaluator_isolation_environment() -> Result<(), String> {
    let Some(path) = configured_dir(CANON_SANDBOX_DIR) else {
        return Ok(());
    };
    filesystem::create_private_dir_all(&path).map_err(|err| {
        format!(
            "failed to prepare {} {}: {}",
            CANON_SANDBOX_DIR,
            path.display(),
            err
        )
    })
}

pub(super) struct SandboxDir {
    pub(super) path: PathBuf,
    pub(super) remove_on_drop: bool,
}

impl Drop for SandboxDir {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn configured_secret_dir() -> Option<PathBuf> {
    env::var_os(CANON_SECRET_DIR).map(PathBuf::from)
}

pub(super) fn configured_dir(name: &str) -> Option<PathBuf> {
    let value = env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

pub(super) fn make_temp_dir() -> Result<PathBuf, String> {
    let parent = env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..1000 {
        let path = parent.join(format!(
            "canon-sandbox-{}-{}-{}",
            process::id(),
            stamp,
            attempt
        ));
        match filesystem::create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create {}: {}", path.display(), err)),
        }
    }
    Err(format!(
        "failed to allocate a unique sandbox directory under {}",
        parent.display()
    ))
}
