use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::git::resolve_git_path;
use crate::history::record::{history_file_name, read_repository_history_records_from_path};
use crate::state_paths::CANON_CACHE_DIR_GIT_PATH;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct HistoryCache {
    pub(crate) cache_dirs: BTreeMap<PathBuf, PathBuf>,
    pub(crate) paths: BTreeMap<(PathBuf, String), PathBuf>,
    pub(crate) records: BTreeMap<HistoryRecordsKey, Vec<CheckRecord>>,
    // `check::order_state` owns the latest-non-pass marker policy; the cache
    // lives here with the other history path/read caches for one run.
    pub(crate) latest_non_pass: BTreeMap<PathBuf, Option<u64>>,
}

pub(super) type HistoryRecordsKey = (PathBuf, String);

impl HistoryCache {
    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        let records_key = history_records_key(&path, &expectation.expected_answer);
        if let Some(records) = self.records.get(&records_key) {
            return Ok(records.clone());
        }
        // Runtime cache reads know the repository root, so this is where
        // answer-history rows are checked against the repository-native Git
        // object hash algorithm. The lower-level line parser only validates the
        // portable JSONL shape used by compaction and parser tests.
        let records =
            read_repository_history_records_from_path(root, &path, &expectation.expected_answer)?;
        self.records.insert(records_key, records.clone());
        Ok(records)
    }

    pub(super) fn record_keys_for_path(&self, path: &Path) -> Vec<HistoryRecordsKey> {
        self.records
            .keys()
            .filter(|(cached_path, _)| cached_path == path)
            .cloned()
            .collect()
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
        let path = resolve_git_path(root, CANON_CACHE_DIR_GIT_PATH)?;
        self.cache_dirs.insert(key, path.clone());
        Ok(path)
    }
}

pub(super) fn history_records_key(path: &Path, expected_answer: &str) -> HistoryRecordsKey {
    (path.to_path_buf(), expected_answer.to_string())
}

pub(super) fn record_for_expected_answer(
    record: &CheckRecord,
    expected_answer: &str,
) -> CheckRecord {
    let mut record = record.clone();
    record.result = CheckResult::from_expected_answer(expected_answer, &record.observed);
    record.expected_answer = Some(expected_answer.to_string());
    record
}
