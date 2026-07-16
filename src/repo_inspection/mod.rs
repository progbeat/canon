use crate::check::CheckConfigSource;
use crate::check::{
    expand_staged_generator_paths_from_listing,
    parse_check_config_content_with_root_and_source_and_default_agent_preset,
    parse_tree_check_config_content_with_root_and_default_agent_preset,
    parse_yaml_config_with_includes, CHECK_PATH,
};
use crate::config_types::{CheckConfig, RawExpectationItem};
use crate::fs_util::reject_symlink;
use crate::git::{read_git_blobs, staged_tracked_files, StagedTrackedFile, TreeSource};
use crate::platform::git_path_bytes;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

type GeneratorPathsCacheKey = (PathBuf, PathBuf, String, String);
type InPlaceFileContentCacheKey = (PathBuf, PathBuf);
type StagedFileContentCacheKey = (PathBuf, PathBuf);
type TreeFileContentCacheKey = (PathBuf, String, PathBuf);
type CheckConfigCacheKey = (
    PathBuf,
    PathBuf,
    String,
    String,
    Option<String>,
    Option<String>,
);
type IncludedExpectationsCacheKey = (PathBuf, String, String, String);
type StagedBlobContents = BTreeMap<Vec<u8>, Vec<u8>>;

#[derive(Default)]
pub(crate) struct RepoInspectionCache {
    generator_paths: BTreeMap<GeneratorPathsCacheKey, Result<Vec<String>, String>>,
    // Per-file decoded content is derived from the root-level staged blob
    // batch below; cache misses here do not spawn additional git processes.
    in_place_file_contents: BTreeMap<InPlaceFileContentCacheKey, Result<String, String>>,
    staged_file_contents: BTreeMap<StagedFileContentCacheKey, Result<String, String>>,
    tree_file_contents: BTreeMap<TreeFileContentCacheKey, Result<String, String>>,
    staged_files: BTreeMap<PathBuf, Result<Vec<StagedTrackedFile>, String>>,
    tree_files: BTreeMap<(PathBuf, String), Result<Vec<StagedTrackedFile>, String>>,
    staged_blob_contents: BTreeMap<PathBuf, Result<StagedBlobContents, String>>,
    tree_blob_contents: BTreeMap<(PathBuf, String), Result<StagedBlobContents, String>>,
    in_place_files: BTreeMap<PathBuf, Result<Vec<String>, String>>,
    check_configs: BTreeMap<CheckConfigCacheKey, Result<CheckConfig, String>>,
    included_expectations:
        BTreeMap<IncludedExpectationsCacheKey, Result<Vec<RawExpectationItem>, String>>,
}

impl RepoInspectionCache {
    pub(crate) fn new() -> RepoInspectionCache {
        RepoInspectionCache::default()
    }

    pub(crate) fn generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        glob: &str,
        source: &CheckConfigSource,
    ) -> Result<Vec<String>, String> {
        let source_key = source.cache_key();
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            glob.to_string(),
            source_key,
        );
        if let Some(cached) = self.generator_paths.get(&key) {
            return cached.clone();
        }
        let expanded = match source {
            CheckConfigSource::Tree(TreeSource::Staged) => {
                self.expand_staged_generator_paths(root, config_path, glob)
            }
            CheckConfigSource::Tree(source) => {
                self.expand_tree_generator_paths(root, config_path, glob, source)
            }
            CheckConfigSource::InPlace => {
                self.expand_in_place_generator_paths(root, config_path, glob)
            }
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
        glob: &str,
    ) -> Result<Vec<String>, String> {
        let staged_paths = self
            .staged_files(root)?
            .into_iter()
            .filter_map(|file| String::from_utf8(file.path).ok())
            .collect::<Vec<_>>();
        expand_staged_generator_paths_from_listing(config_path, glob, &staged_paths)
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

    pub(crate) fn tree_file_content(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: impl AsRef<Path>,
    ) -> Result<String, String> {
        match source {
            TreeSource::Staged => self.staged_file_content(root, path),
            TreeSource::Git { .. } => {
                let path = path.as_ref();
                let key = (root.to_path_buf(), source.cache_key(), path.to_path_buf());
                if let Some(cached) = self.tree_file_contents.get(&key) {
                    return cached.clone();
                }
                let content = self.tree_file_content_from_batch(root, source, path);
                self.tree_file_contents.insert(key, content.clone());
                content
            }
        }
    }

    pub(crate) fn config_source_file_content(
        &mut self,
        root: &Path,
        source: &CheckConfigSource,
        path: impl AsRef<Path>,
    ) -> Result<String, String> {
        match source {
            CheckConfigSource::Tree(source) => self.tree_file_content(root, source, path),
            CheckConfigSource::InPlace => self.in_place_file_content(root, path.as_ref()),
        }
    }

    fn in_place_file_content(&mut self, root: &Path, path: &Path) -> Result<String, String> {
        let key = (root.to_path_buf(), path.to_path_buf());
        if let Some(cached) = self.in_place_file_contents.get(&key) {
            return cached.clone();
        }
        let content = in_place_file_content_from_fs(root, path);
        self.in_place_file_contents.insert(key, content.clone());
        content
    }

    fn expand_tree_generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        glob: &str,
        source: &TreeSource,
    ) -> Result<Vec<String>, String> {
        let tree_paths = self
            .tree_files(root, source)?
            .into_iter()
            .filter(|file| file.is_blob_file_entry())
            .filter_map(|file| String::from_utf8(file.path).ok())
            .collect::<Vec<_>>();
        expand_staged_generator_paths_from_listing(config_path, glob, &tree_paths)
    }

    fn expand_in_place_generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        glob: &str,
    ) -> Result<Vec<String>, String> {
        let files = self.in_place_files(root)?;
        expand_staged_generator_paths_from_listing(config_path, glob, &files)
    }

    fn tree_file_content_from_batch(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: &Path,
    ) -> Result<String, String> {
        let raw_path = git_path_bytes(path)?;
        let contents = self.tree_blob_contents(root, source)?;
        let content = contents.get(&raw_path).ok_or_else(|| {
            format!(
                "failed to read {} from {}: path is not in the selected tree",
                path.display(),
                source.cache_key()
            )
        })?;
        String::from_utf8(content.clone())
            .map_err(|_| format!("tree {} must be valid UTF-8", path.display()))
    }

    fn staged_files(&mut self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        if let Some(cached) = self.staged_files.get(root) {
            return cached.clone();
        }
        let files = staged_tracked_files(root);
        self.staged_files.insert(root.to_path_buf(), files.clone());
        files
    }

    fn tree_files(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<StagedTrackedFile>, String> {
        let key = (root.to_path_buf(), source.cache_key());
        if let Some(cached) = self.tree_files.get(&key) {
            return cached.clone();
        }
        let files = source.tracked_files(root);
        self.tree_files.insert(key, files.clone());
        files
    }

    fn in_place_files(&mut self, root: &Path) -> Result<Vec<String>, String> {
        if let Some(cached) = self.in_place_files.get(root) {
            return cached.clone();
        }
        let files = in_place_file_listing(root);
        self.in_place_files
            .insert(root.to_path_buf(), files.clone());
        files
    }

    fn staged_blob_contents(&mut self, root: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        if let Some(cached) = self.staged_blob_contents.get(root) {
            return cached.clone();
        }
        let files = self.staged_files(root)?;
        let blob_files = files
            .iter()
            .filter(|file| file.is_blob_file_entry())
            .cloned()
            .collect::<Vec<_>>();
        let object_ids = blob_files
            .iter()
            .map(|file| file.object_id.clone())
            .collect::<Vec<_>>();
        let blobs = read_git_blobs(root, &object_ids)?;
        let contents = blob_files
            .into_iter()
            .zip(blobs)
            .map(|(file, blob)| (file.path, blob))
            .collect::<BTreeMap<_, _>>();
        let result = Ok(contents);
        self.staged_blob_contents
            .insert(root.to_path_buf(), result.clone());
        result
    }

    fn tree_blob_contents(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        match source {
            TreeSource::Staged => self.staged_blob_contents(root),
            TreeSource::Git { .. } => {
                let key = (root.to_path_buf(), source.cache_key());
                if let Some(cached) = self.tree_blob_contents.get(&key) {
                    return cached.clone();
                }
                let files = self.tree_files(root, source)?;
                let blob_files = files
                    .iter()
                    .filter(|file| file.is_blob_file_entry())
                    .cloned()
                    .collect::<Vec<_>>();
                let object_ids = blob_files
                    .iter()
                    .map(|file| file.object_id.clone())
                    .collect::<Vec<_>>();
                let blobs = read_git_blobs(root, &object_ids)?;
                let contents = blob_files
                    .into_iter()
                    .zip(blobs)
                    .map(|(file, blob)| (file.path, blob))
                    .collect::<BTreeMap<_, _>>();
                let result = Ok(contents);
                self.tree_blob_contents.insert(key, result.clone());
                result
            }
        }
    }

    pub(crate) fn load_check_config(
        &mut self,
        root: &Path,
        config_path: &Path,
        source: &TreeSource,
    ) -> Result<CheckConfig, String> {
        self.load_check_config_with_default_agent_preset(root, config_path, source, None)
    }

    pub(crate) fn load_check_config_with_default_agent_preset(
        &mut self,
        root: &Path,
        config_path: &Path,
        source: &TreeSource,
        default_agent_preset: Option<&str>,
    ) -> Result<CheckConfig, String> {
        self.load_tree_check_config(root, config_path, source, default_agent_preset, None)
    }

    pub(crate) fn load_ask_config(
        &mut self,
        root: &Path,
        config_path: &Path,
        source: &TreeSource,
        default_agent_preset: Option<&str>,
        question: &str,
    ) -> Result<CheckConfig, String> {
        self.load_tree_check_config(
            root,
            config_path,
            source,
            default_agent_preset,
            Some(question),
        )
    }

    fn load_tree_check_config(
        &mut self,
        root: &Path,
        config_path: &Path,
        source: &TreeSource,
        default_agent_preset: Option<&str>,
        ask_question: Option<&str>,
    ) -> Result<CheckConfig, String> {
        let content = self.tree_file_content(root, source, config_path)?;
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            content.clone(),
            source.cache_key(),
            default_agent_preset.map(str::to_string),
            ask_question.map(str::to_string),
        );
        if let Some(cached) = self.check_configs.get(&key) {
            return cached.clone();
        }
        let parsed = parse_tree_check_config_content_with_root_and_default_agent_preset(
            root,
            config_path,
            &content,
            self,
            source.clone(),
            default_agent_preset,
            ask_question,
        );
        self.check_configs.insert(key, parsed.clone());
        parsed
    }

    pub(crate) fn load_in_place_check_config_with_default_agent_preset(
        &mut self,
        root: &Path,
        config_path: &Path,
        default_agent_preset: Option<&str>,
    ) -> Result<CheckConfig, String> {
        self.load_in_place_check_config(root, config_path, default_agent_preset, None)
    }

    pub(crate) fn load_in_place_ask_config(
        &mut self,
        root: &Path,
        config_path: &Path,
        default_agent_preset: Option<&str>,
        question: &str,
    ) -> Result<CheckConfig, String> {
        self.load_in_place_check_config(root, config_path, default_agent_preset, Some(question))
    }

    fn load_in_place_check_config(
        &mut self,
        root: &Path,
        config_path: &Path,
        default_agent_preset: Option<&str>,
        ask_question: Option<&str>,
    ) -> Result<CheckConfig, String> {
        let source = CheckConfigSource::InPlace;
        let content = self.in_place_file_content(root, config_path)?;
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            content.clone(),
            source.cache_key(),
            default_agent_preset.map(str::to_string),
            ask_question.map(str::to_string),
        );
        if let Some(cached) = self.check_configs.get(&key) {
            return cached.clone();
        }
        let parsed = parse_check_config_content_with_root_and_source_and_default_agent_preset(
            root,
            config_path,
            &content,
            self,
            source,
            default_agent_preset,
            ask_question,
        );
        self.check_configs.insert(key, parsed.clone());
        parsed
    }

    pub(crate) fn included_expectation_items(
        &mut self,
        root: &Path,
        source: &CheckConfigSource,
        file: &str,
        content: &str,
    ) -> Result<Vec<RawExpectationItem>, String> {
        let key = (
            root.to_path_buf(),
            source.cache_key(),
            file.to_string(),
            content.to_string(),
        );
        if let Some(cached) = self.included_expectations.get(&key) {
            return cached.clone();
        }
        let parsed =
            parse_yaml_config_with_includes(root, Path::new(file), content, source.clone())
                .map_err(|err| format!("failed to parse {}: {}", file, err));
        self.included_expectations.insert(key, parsed.clone());
        parsed
    }
}

fn in_place_file_content_from_fs(root: &Path, path: &Path) -> Result<String, String> {
    let path = root.join(path);
    reject_symlink(&path)?;
    fs::read_to_string(&path).map_err(|err| format!("failed to read {}: {}", path.display(), err))
}

fn in_place_file_listing(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_in_place_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_in_place_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {}", dir.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        // [Df] In-place inspects filesystem project contents without consuming
        // Git repository metadata. `.gitignore` and other ordinary project
        // files remain visible; only metadata entries named `.git` are skipped.
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
        if file_type.is_dir() {
            collect_in_place_files(root, &path, files)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| format!("failed to relativize {}", path.display()))?;
            if let Some(path) = relative.to_str() {
                files.push(path.replace(std::path::MAIN_SEPARATOR, "/"));
            }
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn in_place_file_content_rejects_symlink() {
        let root = test_root("in-place-file-content-rejects-symlink");
        let outside = outside_test_file(&root);
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("config.yml")).unwrap();

        let error = in_place_file_content_from_fs(&root, Path::new("config.yml")).unwrap_err();

        assert!(error.contains("refusing to use symlink"));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn in_place_file_listing_includes_symlinks() {
        let root = test_root("in-place-file-listing-includes-symlinks");
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(root.join("specs/real.md"), "real").unwrap();
        let outside = outside_test_file(&root);
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("specs/link.md")).unwrap();

        let files = in_place_file_listing(&root).unwrap();

        assert_eq!(files, vec!["specs/link.md", "specs/real.md"]);
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Df
    fn in_place_file_listing_ignores_git_metadata_only() {
        let root = test_root("in-place-file-listing-ignores-git-metadata");
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let files = in_place_file_listing(&root).unwrap();

        assert_eq!(
            files,
            vec![".gitignore", "src/main.rs"],
            "in-place listing must ignore Git metadata without hiding project files"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn outside_test_file(root: &Path) -> PathBuf {
        let file_name = root
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or("canon-test");
        root.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{file_name}-outside"))
    }
}
