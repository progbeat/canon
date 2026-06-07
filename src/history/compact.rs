use crate::fs_util::write_temp_file_then_replace;
use crate::git::{git_object_oid_has_hex_len, VisibleTreeOidCache};
use crate::history::record::parse_history_record_line;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

static HISTORY_COMPACT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const HISTORY_COMPACT_KEEP_RECORDS: usize = 8;
const HISTORY_COMPACT_CHANCE_DENOMINATOR: u64 = 16;

pub(super) fn should_compact_history() -> bool {
    getrandom::u64().is_ok_and(should_compact_history_for_random_draw)
}

pub(super) fn should_compact_history_for_random_draw(draw: u64) -> bool {
    draw.is_multiple_of(HISTORY_COMPACT_CHANCE_DENOMINATOR)
}

pub(super) fn compact_repository_history_locked(root: &Path, path: &Path) -> Result<(), String> {
    let native_oid_hex_len =
        VisibleTreeOidCache::new().repository_native_object_oid_hex_len(root)?;
    compact_history_locked_with_native_oid_len(path, Some(native_oid_hex_len))
}

fn compact_history_locked_with_native_oid_len(
    path: &Path,
    native_oid_hex_len: Option<usize>,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let text = String::from_utf8(bytes).map_err(|err| {
        format!(
            "history file must be valid UTF-8 ({}): {}",
            path.display(),
            err
        )
    })?;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if valid_history_record_line(path, index + 1, line, native_oid_hex_len) {
            lines.push(line.to_string());
        }
    }
    if lines.len() <= HISTORY_COMPACT_KEEP_RECORDS {
        return Ok(());
    }
    let start = lines.len() - HISTORY_COMPACT_KEEP_RECORDS;
    let retained = &lines[start..];
    let temp_path = compact_history_temp_path(path)?;
    write_temp_file_then_replace(&temp_path, path, |file| {
        for line in retained {
            file.write_all(line.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            file.write_all(b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })
    .map_err(|err| err.to_string())
}

fn valid_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
    native_oid_hex_len: Option<usize>,
) -> bool {
    let Ok(record) = parse_history_record_line(path, line_number, line) else {
        return false;
    };
    native_oid_hex_len
        .is_none_or(|hex_len| git_object_oid_has_hex_len(&record.visible_tree_oid, hex_len))
}

pub(super) fn compact_history_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("history path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = HISTORY_COMPACT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
