use super::{cached_inspection, RepoInspectionCache};
use crate::git::{read_git_blobs, staged_tracked_files, TrackedFile, TreeSource};
use crate::platform::filesystem::git_path_bytes;
use std::path::Path;
use std::sync::Arc;

impl RepoInspectionCache {
    pub(crate) fn staged_file_content(
        &mut self,
        root: &Path,
        path: impl AsRef<Path>,
    ) -> Result<String, String> {
        let path = path.as_ref();
        let key = (root.to_path_buf(), path.to_path_buf());
        let state = Arc::clone(&self.state);
        cached_inspection(
            &state,
            key,
            |state| &state.staged_file_contents,
            |state| &mut state.staged_file_contents,
            || self.staged_file_content_from_git(root, path),
        )
    }

    pub(crate) fn tree_blob_paths(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<Vec<u8>>, String> {
        Ok(blob_paths_from_tracked_files(
            self.git_tracked_files(root, source)?,
        ))
    }

    fn staged_file_content_from_git(&mut self, root: &Path, path: &Path) -> Result<String, String> {
        self.tracked_file_content_from_git(
            root,
            &TreeSource::Staged,
            path,
            format!(
                "failed to read staged {}: path is not in the staged index",
                path.display()
            ),
            format!("staged {} must be valid UTF-8", path.display()),
        )
    }

    pub(crate) fn tree_file_content(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: impl AsRef<Path>,
    ) -> Result<String, String> {
        match source {
            TreeSource::Staged => self.staged_file_content(root, path),
            TreeSource::Git { .. }
            | TreeSource::TemporaryGit { .. }
            | TreeSource::DefaultAgainstHead { .. }
            | TreeSource::DefaultAgainstUnbornHead { .. } => {
                let path = path.as_ref();
                let key = (root.to_path_buf(), source.cache_key(), path.to_path_buf());
                let state = Arc::clone(&self.state);
                cached_inspection(
                    &state,
                    key,
                    |state| &state.tree_file_contents,
                    |state| &mut state.tree_file_contents,
                    || self.tree_file_content_from_git(root, source, path),
                )
            }
        }
    }

    pub(crate) fn in_place_file_content(
        &mut self,
        root: &Path,
        path: &Path,
    ) -> Result<String, String> {
        let key = (root.to_path_buf(), path.to_path_buf());
        cached_inspection(
            &self.state,
            key,
            |state| &state.in_place_file_contents,
            |state| &mut state.in_place_file_contents,
            || super::in_place::file_content(root, path),
        )
    }

    fn tree_file_content_from_git(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: &Path,
    ) -> Result<String, String> {
        self.tracked_file_content_from_git(
            root,
            source,
            path,
            format!(
                "failed to read {} from {}: path is not in the selected tree",
                path.display(),
                source.cache_key()
            ),
            format!("tree {} must be valid UTF-8", path.display()),
        )
    }

    fn tracked_file_content_from_git(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: &Path,
        missing_path_error: String,
        invalid_utf8_error: String,
    ) -> Result<String, String> {
        let raw_path = git_path_bytes(path)?;
        let files = self.git_tracked_files(root, source)?;
        let content = tracked_blob_content(root, &files, &raw_path, missing_path_error)?;
        String::from_utf8(content).map_err(|_| invalid_utf8_error)
    }

    pub(crate) fn git_tracked_files(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<TrackedFile>, String> {
        match source {
            TreeSource::Staged => self.staged_files(root),
            TreeSource::Git { .. }
            | TreeSource::TemporaryGit { .. }
            | TreeSource::DefaultAgainstHead { .. }
            | TreeSource::DefaultAgainstUnbornHead { .. } => self.tree_files(root, source),
        }
    }

    pub(crate) fn git_tracked_files_for_pathspecs(
        &mut self,
        root: &Path,
        source: &TreeSource,
        pathspecs: &[String],
    ) -> Result<Vec<TrackedFile>, String> {
        let key = (root.to_path_buf(), source.cache_key(), pathspecs.to_vec());
        cached_inspection(
            &self.state,
            key,
            |state| &state.pathspec_files,
            |state| &mut state.pathspec_files,
            || source.tracked_files_for_pathspecs(root, pathspecs),
        )
    }

    fn staged_files(&mut self, root: &Path) -> Result<Vec<TrackedFile>, String> {
        // `self` identifies the invocation snapshot. Within it, the repository
        // root fully identifies the one staged-index observation.
        cached_inspection(
            &self.state,
            root.to_path_buf(),
            |state| &state.staged_files,
            |state| &mut state.staged_files,
            || staged_tracked_files(root),
        )
    }

    fn tree_files(&mut self, root: &Path, source: &TreeSource) -> Result<Vec<TrackedFile>, String> {
        let key = (root.to_path_buf(), source.cache_key());
        cached_inspection(
            &self.state,
            key,
            |state| &state.tree_files,
            |state| &mut state.tree_files,
            || source.tracked_files(root),
        )
    }

    pub(crate) fn in_place_file_paths(&mut self, root: &Path) -> Result<Vec<Vec<u8>>, String> {
        cached_inspection(
            &self.state,
            root.to_path_buf(),
            |state| &state.in_place_files,
            |state| &mut state.in_place_files,
            || super::in_place::file_listing(root),
        )
    }
}

fn tracked_blob_content(
    root: &Path,
    files: &[TrackedFile],
    raw_path: &[u8],
    missing_message: String,
) -> Result<Vec<u8>, String> {
    // [t] A config read must not materialize unrelated repository blobs:
    // their size is outside the bounded configuration being parsed.
    let object_id = files
        .iter()
        .find(|file| file.is_blob_file_entry() && file.path == raw_path)
        .map(|file| file.object_id.clone())
        .ok_or(missing_message)?;
    read_git_blobs(root, std::slice::from_ref(&object_id))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("git cat-file returned no content for blob {object_id}"))
}

fn blob_paths_from_tracked_files(files: Vec<TrackedFile>) -> Vec<Vec<u8>> {
    files
        .into_iter()
        .filter(|file| file.is_blob_file_entry())
        .map(|file| file.path)
        .collect()
}
