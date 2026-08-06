use crate::memoize::{mutex_memoized_result, MemoizedResult};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ProjectInspectionCacheKey {
    pub(super) cwd: PathBuf,
    pub(super) template_artifact_directory: PathBuf,
    pub(super) tool: String,
    pub(super) arguments: String,
}

#[derive(Default)]
pub(super) struct EvaluatorProjectInspectionCache {
    entries: Mutex<BTreeMap<ProjectInspectionCacheKey, MemoizedResult<String>>>,
}

impl EvaluatorProjectInspectionCache {
    pub(super) fn result(
        &self,
        key: ProjectInspectionCacheKey,
        compute: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        mutex_memoized_result(
            &self.entries,
            key,
            "evaluator project inspection cache lock is poisoned",
            |entries| entries,
            |entries| entries,
            compute,
        )
    }
}
