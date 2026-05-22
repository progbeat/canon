use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::history::HistoryCache;
use crate::time::parse_record_timestamp;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LATEST_NON_PASS_FILE: &str = "latest-non-pass.json";

#[cfg(test)]
pub(crate) fn latest_recorded_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Option<u64>, String> {
    let mut history_cache = HistoryCache::new();
    latest_recorded_non_pass_timestamp_with_cache(root, expectation, &mut history_cache)
}

pub(crate) fn latest_recorded_non_pass_timestamp_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<u64>, String> {
    let path = latest_non_pass_path(root, expectation, history_cache)?;
    if let Some(timestamp) = history_cache.latest_non_pass.get(&path) {
        return Ok(*timestamp);
    }
    reject_symlink(&path)?;
    if !path.exists() {
        history_cache.latest_non_pass.insert(path, None);
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let record = serde_json::from_str::<LatestNonPassRecord>(&content).map_err(|err| {
        format!(
            "invalid latest non-pass JSON in {}: {}",
            path.display(),
            err
        )
    })?;
    let timestamp = parse_record_timestamp(&record.timestamp);
    history_cache.latest_non_pass.insert(path, timestamp);
    Ok(timestamp)
}

#[cfg(test)]
pub(crate) fn write_latest_non_pass_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    let mut history_cache = HistoryCache::new();
    write_latest_non_pass_record_with_cache(root, expectation, record, &mut history_cache)
}

pub(crate) fn write_latest_non_pass_record_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    if record.passed() {
        return Ok(());
    }
    let path = latest_non_pass_path(root, expectation, history_cache)?;
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(&path)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    // Keep this bounded order state separate from answer history and runtime
    // logs: answer history excludes human-review records, and runtime logs are
    // diagnostic output rather than input to future command behavior.
    let mut line = json!({
        "timestamp": record.timestamp,
        "result": record.result,
        "observed": record.observed,
    })
    .to_string();
    line.push('\n');
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))?;
    history_cache
        .latest_non_pass
        .insert(path, parse_record_timestamp(&record.timestamp));
    Ok(())
}

fn latest_non_pass_path(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<PathBuf, String> {
    Ok(history_cache
        .cache_dir(root)?
        .join(&expectation.id)
        .join(LATEST_NON_PASS_FILE))
}

#[derive(Deserialize)]
struct LatestNonPassRecord {
    timestamp: String,
}
