use crate::git::{TrackedFile, TreeSource};
use crate::repo_inspection::RepoInspectionCache;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(super) fn read(
    repo_cache: &mut RepoInspectionCache,
    root: &Path,
    baseline: &TreeSource,
    staged: &TreeSource,
) -> Result<Vec<Vec<u8>>, String> {
    if matches!(baseline, TreeSource::Staged) || matches!(staged, TreeSource::Staged) {
        return Err("gate changed-path inspection requires OID-backed prepared trees".to_string());
    }
    let baseline = entries_by_path(repo_cache.git_tracked_files(root, baseline)?);
    let staged = entries_by_path(repo_cache.git_tracked_files(root, staged)?);
    let paths = baseline
        .keys()
        .chain(staged.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(paths
        .into_iter()
        .filter(|path| baseline.get(path) != staged.get(path))
        .collect())
}

fn entries_by_path(files: Vec<TrackedFile>) -> BTreeMap<Vec<u8>, (String, String)> {
    files
        .into_iter()
        .map(|file| (file.path, (file.mode, file.object_id)))
        .collect()
}
