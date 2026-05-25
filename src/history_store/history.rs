use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::fs_util::for_each_nonempty_line;
use crate::git::resolve_git_path;
use crate::GIT_CANON_CACHE_DIR;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) fn history_path(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<PathBuf, String> {
    Ok(resolve_git_path(root, GIT_CANON_CACHE_DIR)?
        .join(&expectation.id)
        .join(history_file_name()))
}

pub(crate) fn history_file_name() -> &'static str {
    "history.jsonl"
}

#[cfg(test)]
pub(crate) fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    read_history_records_from_path(&path)
}

pub(crate) fn read_history_records_from_path(path: &Path) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        match parse_history_record_line(path, line_number, &line) {
            Ok(record) => records.push(record),
            Err(_) => {
                // History is a reusable cache, not authoritative project data.
                // Corrupt cache lines are ignored here and dropped by the same
                // parser during compaction, while real file I/O errors still
                // propagate from `for_each_nonempty_line`.
            }
        }
        Ok(())
    })?;
    Ok(records)
}

pub(crate) fn parse_history_record_line(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<CheckRecord, String> {
    serde_json::from_str::<CheckRecord>(line).map_err(|err| {
        format!(
            "invalid history JSON in {} line {}: {}",
            path.display(),
            line_number,
            err
        )
    })
}

#[derive(Default)]
pub(crate) struct HistoryCache {
    pub(crate) cache_dirs: BTreeMap<PathBuf, PathBuf>,
    pub(crate) paths: BTreeMap<(PathBuf, String), PathBuf>,
    pub(crate) records: BTreeMap<PathBuf, Vec<CheckRecord>>,
    pub(crate) latest_non_pass: BTreeMap<PathBuf, Option<u64>>,
}

impl HistoryCache {
    pub(crate) fn new() -> HistoryCache {
        HistoryCache::default()
    }

    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        if let Some(records) = self.records.get(&path) {
            return Ok(records.clone());
        }
        let records = read_history_records_from_path(&path)?;
        self.records.insert(path, records.clone());
        Ok(records)
    }

    pub(crate) fn path(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.paths.get(&key) {
            return Ok(path.clone());
        }
        let path = self
            .cache_dir(root)?
            .join(&expectation.id)
            .join(history_file_name());
        self.paths.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn cache_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.cache_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(root, GIT_CANON_CACHE_DIR)?;
        self.cache_dirs.insert(key, path.clone());
        Ok(path)
    }
}
