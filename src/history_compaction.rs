use crate::fs_util::{for_each_nonempty_line, write_temp_file_then_replace};
use crate::history::parse_history_record_line;
use crate::{HISTORY_COMPACT_CHANCE_DENOMINATOR, HISTORY_COMPACT_KEEP_RECORDS};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static HISTORY_COMPACT_CHANCE_COUNTER: AtomicU64 = AtomicU64::new(0);
static HISTORY_COMPACT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn should_compact_history() -> bool {
    should_compact_history_for_seed(compaction_chance_seed())
}

pub(crate) fn should_compact_history_for_seed(seed: u64) -> bool {
    seed.is_multiple_of(HISTORY_COMPACT_CHANCE_DENOMINATOR)
}

fn compaction_chance_seed() -> u64 {
    let counter = HISTORY_COMPACT_CHANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ counter.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ process::id() as u64
}

pub(crate) fn compact_history(path: &Path) -> Result<(), String> {
    let mut valid_lines = 0usize;
    let mut invalid_lines = 0usize;
    let mut lines = std::collections::VecDeque::new();
    for_each_nonempty_line(path, |line_number, line| {
        if parse_history_record_line(path, line_number, &line).is_ok() {
            valid_lines += 1;
            lines.push_back(line);
            if lines.len() > HISTORY_COMPACT_KEEP_RECORDS {
                lines.pop_front();
            }
        } else {
            invalid_lines += 1;
        }
        Ok(())
    })?;
    if valid_lines <= HISTORY_COMPACT_KEEP_RECORDS && invalid_lines == 0 {
        return Ok(());
    }
    let temp_path = compact_history_temp_path(path)?;
    write_temp_file_then_replace(&temp_path, path, |file| {
        for line in lines {
            file.write_all(line.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
            file.write_all(b"\n")
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })
}

pub(crate) fn compact_history_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("history path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    let sequence = HISTORY_COMPACT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    temp_name.push(format!(".tmp.{}.{}", process::id(), sequence));
    Ok(path.with_file_name(temp_name))
}
