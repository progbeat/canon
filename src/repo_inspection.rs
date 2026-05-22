use crate::check_config::parse_staged_check_config_content_with_root;
use crate::check_generator_paths::{
    expand_generator_paths, expand_staged_generator_paths_from_listing,
};
use crate::config_types::{CheckConfig, RawExpectationItem};
use crate::git::{
    git_path_bytes, read_git_blobs, resolve_git_path, staged_tracked_files, StagedTrackedFile,
};
use crate::CHECK_PATH;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

type GitPathCacheKey = (PathBuf, String);
type GeneratorPathsCacheKey = (PathBuf, PathBuf, String, bool);
type StagedFileContentCacheKey = (PathBuf, PathBuf);
type CheckConfigCacheKey = (PathBuf, PathBuf, String);
type IncludedExpectationsCacheKey = (String, String);
type StagedBlobContents = BTreeMap<Vec<u8>, Vec<u8>>;

#[derive(Default)]
pub(crate) struct RepoInspectionCache {
    git_paths: BTreeMap<GitPathCacheKey, Result<PathBuf, String>>,
    generator_paths: BTreeMap<GeneratorPathsCacheKey, Result<Vec<String>, String>>,
    staged_file_contents: BTreeMap<StagedFileContentCacheKey, Result<String, String>>,
    staged_files: BTreeMap<PathBuf, Result<Vec<StagedTrackedFile>, String>>,
    staged_blob_contents: BTreeMap<PathBuf, Result<StagedBlobContents, String>>,
    #[cfg(test)]
    filesystem_text: BTreeMap<PathBuf, Result<String, String>>,
    check_configs: BTreeMap<CheckConfigCacheKey, Result<CheckConfig, String>>,
    included_expectations:
        BTreeMap<IncludedExpectationsCacheKey, Result<Vec<RawExpectationItem>, String>>,
}

impl RepoInspectionCache {
    pub(crate) fn new() -> RepoInspectionCache {
        RepoInspectionCache::default()
    }

    pub(crate) fn git_path(&mut self, root: &Path, path: &str) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), path.to_string());
        if let Some(cached) = self.git_paths.get(&key) {
            return cached.clone();
        }
        let resolved = resolve_git_path(root, path);
        self.git_paths.insert(key, resolved.clone());
        resolved
    }

    pub(crate) fn generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        path: &str,
        staged: bool,
    ) -> Result<Vec<String>, String> {
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            path.to_string(),
            staged,
        );
        if let Some(cached) = self.generator_paths.get(&key) {
            return cached.clone();
        }
        let expanded = if staged {
            self.expand_staged_generator_paths(root, config_path, path)
        } else {
            expand_generator_paths(root, config_path, path, false)
        };
        self.generator_paths.insert(key, expanded.clone());
        expanded
    }

    pub(crate) fn staged_file_content(
        &mut self,
        root: &Path,
        path: impl AsRef<Path>,
    ) -> Result<String, String> {
        let path = path.as_ref();
        let key = (root.to_path_buf(), path.to_path_buf());
        if let Some(cached) = self.staged_file_contents.get(&key) {
            return cached.clone();
        }
        let content = self.staged_file_content_from_batch(root, path);
        self.staged_file_contents.insert(key, content.clone());
        content
    }

    fn expand_staged_generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        path: &str,
    ) -> Result<Vec<String>, String> {
        let staged_paths = self
            .staged_files(root)?
            .into_iter()
            .filter_map(|file| String::from_utf8(file.path).ok())
            .collect::<Vec<_>>();
        expand_staged_generator_paths_from_listing(config_path, path, &staged_paths)
    }

    fn staged_file_content_from_batch(
        &mut self,
        root: &Path,
        path: &Path,
    ) -> Result<String, String> {
        let raw_path = git_path_bytes(path)?;
        let contents = self.staged_blob_contents(root)?;
        let content = contents
            .get(&raw_path)
            .ok_or_else(|| missing_staged_file_message(path))?;
        String::from_utf8(content.clone())
            .map_err(|_| format!("staged {} must be valid UTF-8", path.display()))
    }

    fn staged_files(&mut self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        if let Some(cached) = self.staged_files.get(root) {
            return cached.clone();
        }
        let files = staged_tracked_files(root);
        self.staged_files.insert(root.to_path_buf(), files.clone());
        files
    }

    fn staged_blob_contents(&mut self, root: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        if let Some(cached) = self.staged_blob_contents.get(root) {
            return cached.clone();
        }
        let files = self.staged_files(root)?;
        let object_ids = files
            .iter()
            .map(|file| file.object_id.clone())
            .collect::<Vec<_>>();
        let blobs = read_git_blobs(root, &object_ids)?;
        let contents = files
            .into_iter()
            .zip(blobs)
            .map(|(file, blob)| (file.path, blob))
            .collect::<BTreeMap<_, _>>();
        let result = Ok(contents);
        self.staged_blob_contents
            .insert(root.to_path_buf(), result.clone());
        result
    }

    #[cfg(test)]
    pub(crate) fn read_to_string(&mut self, path: &Path) -> Result<String, String> {
        let key = path.to_path_buf();
        if let Some(cached) = self.filesystem_text.get(&key) {
            return cached.clone();
        }
        let content = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err));
        self.filesystem_text.insert(key, content.clone());
        content
    }

    pub(crate) fn load_check_config(
        &mut self,
        root: &Path,
        config_path: &Path,
    ) -> Result<CheckConfig, String> {
        let content = self.staged_file_content(root, config_path)?;
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            content.clone(),
        );
        if let Some(cached) = self.check_configs.get(&key) {
            return cached.clone();
        }
        let parsed = parse_staged_check_config_content_with_root(root, config_path, &content, self);
        self.check_configs.insert(key, parsed.clone());
        parsed
    }

    pub(crate) fn included_expectation_items(
        &mut self,
        file: &str,
        content: &str,
    ) -> Result<Vec<RawExpectationItem>, String> {
        let key = (file.to_string(), content.to_string());
        if let Some(cached) = self.included_expectations.get(&key) {
            return cached.clone();
        }
        let parsed = serde_saphyr::from_str(content)
            .map_err(|err| format!("failed to parse {}: {}", file, err));
        self.included_expectations.insert(key, parsed.clone());
        parsed
    }
}

fn missing_staged_file_message(path: &Path) -> String {
    if path == Path::new(CHECK_PATH) {
        return format!(
            "No canon check config found at {CHECK_PATH}\n\
             ▷ If the human asked you to use any `canon-*` skill in this project, \
             treat the missing config as permission to set up canon rather than asking \
             for confirmation. Run `canon init`, inspect the current project enough to \
             understand its purpose, public behavior, and important invariants, then \
             draft a minimal `.canon/check.yml` that the human can grow over time. Use \
             canon's own `.canon/check.yml` as a reference for structure and style: \
             `https://raw.githubusercontent.com/progbeat/canon/main/.canon/check.yml`. \
             Start with a few simple, objective expectations that protect important \
             user-facing behavior."
        );
    }
    format!(
        "failed to read staged {}: path is not in the staged index",
        path.display()
    )
}
