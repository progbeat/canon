use super::model::GateHistory;
use crate::fs_util::{reject_symlink, write_temp_file_then_replace};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::xpec_state) const CACHE_FILE_NAME: &str = "git-backed-results.json";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn load_history_file(path: &Path) -> Result<Option<GateHistory>, String> {
    // The safety check is adjacent to the only filesystem read on this path;
    // invocation-local cache hits never access the path at all.
    reject_symlink(path)?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to read {}: {}", path.display(), err)),
    };
    serde_json::from_str::<GateHistory>(&content)
        .map(Some)
        .map_err(|err| {
            format!(
                "invalid Git-backed result cache in {}: {}",
                path.display(),
                err
            )
        })
}

pub(super) fn persist_cache_path(path: &Path, cache: &GateHistory) -> Result<(), String> {
    let temp_path = temp_path(path)?;
    write_temp_file_then_replace(&temp_path, path, |file| {
        serde_json::to_writer(&mut *file, cache)
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        std::io::Write::write_all(file, b"\n")
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
    })
}

fn temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("state path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
