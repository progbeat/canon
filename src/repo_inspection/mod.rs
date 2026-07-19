use crate::git::{read_git_blobs, staged_tracked_files, StagedTrackedFile, TreeSource};
use crate::platform::git_path_bytes;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

type InPlaceFileContentCacheKey = (PathBuf, PathBuf);
type StagedFileContentCacheKey = (PathBuf, PathBuf);
type TreeFileContentCacheKey = (PathBuf, String, PathBuf);

#[derive(Default)]
pub(crate) struct RepoInspectionCache {
    in_place_file_contents: BTreeMap<InPlaceFileContentCacheKey, Result<String, String>>,
    staged_file_contents: BTreeMap<StagedFileContentCacheKey, Result<String, String>>,
    tree_file_contents: BTreeMap<TreeFileContentCacheKey, Result<String, String>>,
    staged_files: BTreeMap<PathBuf, Result<Vec<StagedTrackedFile>, String>>,
    tree_files: BTreeMap<(PathBuf, String), Result<Vec<StagedTrackedFile>, String>>,
    in_place_files: BTreeMap<PathBuf, Result<Vec<Vec<u8>>, String>>,
}

impl RepoInspectionCache {
    pub(crate) fn new() -> RepoInspectionCache {
        RepoInspectionCache::default()
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
        let content = self.staged_file_content_from_git(root, path);
        self.staged_file_contents.insert(key, content.clone());
        content
    }

    pub(crate) fn tree_blob_paths(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<Vec<u8>>, String> {
        match source {
            TreeSource::Staged => Ok(blob_paths_from_tracked_files(self.staged_files(root)?)),
            source => Ok(blob_paths_from_tracked_files(
                self.tree_files(root, source)?,
            )),
        }
    }

    fn staged_file_content_from_git(&mut self, root: &Path, path: &Path) -> Result<String, String> {
        let raw_path = git_path_bytes(path)?;
        let files = self.staged_files(root)?;
        let content = tracked_blob_content(
            root,
            &files,
            &raw_path,
            format!(
                "failed to read staged {}: path is not in the staged index",
                path.display()
            ),
        )?;
        String::from_utf8(content)
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
            TreeSource::Git { .. } | TreeSource::DefaultAgainstHead { .. } => {
                let path = path.as_ref();
                let key = (root.to_path_buf(), source.cache_key(), path.to_path_buf());
                if let Some(cached) = self.tree_file_contents.get(&key) {
                    return cached.clone();
                }
                let content = self.tree_file_content_from_git(root, source, path);
                self.tree_file_contents.insert(key, content.clone());
                content
            }
        }
    }

    pub(crate) fn in_place_file_content(
        &mut self,
        root: &Path,
        path: &Path,
    ) -> Result<String, String> {
        let key = (root.to_path_buf(), path.to_path_buf());
        if let Some(cached) = self.in_place_file_contents.get(&key) {
            return cached.clone();
        }
        let content = in_place_file_content_from_fs(root, path);
        self.in_place_file_contents.insert(key, content.clone());
        content
    }

    fn tree_file_content_from_git(
        &mut self,
        root: &Path,
        source: &TreeSource,
        path: &Path,
    ) -> Result<String, String> {
        let raw_path = git_path_bytes(path)?;
        let files = self.tree_files(root, source)?;
        let content = tracked_blob_content(root, &files, &raw_path, {
            format!(
                "failed to read {} from {}: path is not in the selected tree",
                path.display(),
                source.cache_key()
            )
        })?;
        String::from_utf8(content)
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

    pub(crate) fn in_place_file_paths(&mut self, root: &Path) -> Result<Vec<Vec<u8>>, String> {
        if let Some(cached) = self.in_place_files.get(root) {
            return cached.clone();
        }
        let files = in_place_file_listing(root);
        self.in_place_files
            .insert(root.to_path_buf(), files.clone());
        files
    }
}

fn tracked_blob_content(
    root: &Path,
    files: &[StagedTrackedFile],
    raw_path: &[u8],
    missing_message: String,
) -> Result<Vec<u8>, String> {
    // [tf] A config read must not materialize unrelated repository blobs:
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

fn blob_paths_from_tracked_files(files: Vec<StagedTrackedFile>) -> Vec<Vec<u8>> {
    files
        .into_iter()
        .filter(|file| file.is_blob_file_entry())
        .map(|file| file.path)
        .collect()
}

fn in_place_file_content_from_fs(root: &Path, path: &Path) -> Result<String, String> {
    let path = root.join(path);
    // [cg,s6] In-place uses ordinary filesystem semantics. A path discovered
    // from this same source remains readable when it is a symlink.
    fs::read_to_string(&path).map_err(|err| format!("failed to read {}: {}", path.display(), err))
}

fn in_place_file_listing(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut files = Vec::new();
    collect_in_place_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_in_place_files(root: &Path, dir: &Path, files: &mut Vec<Vec<u8>>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {}", dir.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        // [I4] This listing is used only to discover filesystem inputs for
        // config `foreach` expansion. It is not the evaluator's filesystem
        // view: the in-place evaluator starts directly in `root`, with no
        // project-file hiding at all. Git exposes repository metadata through
        // an entry named `.git`; ignoring that metadata here prevents it from
        // becoming config input. The entry may be either a metadata directory
        // or a gitfile pointing elsewhere, so the name check deliberately runs
        // before file-type inspection and excludes both forms. Project files
        // such as `.gitignore` remain ordinary config inputs and evaluator-
        // visible filesystem contents.
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
            files.push(git_path_bytes(relative)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: tf,dx
    fn staged_file_content_does_not_read_unrelated_blobs() {
        let root = test_root("staged-file-content-requested-blob-only");
        let init = process::Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        fs::write(root.join("config.yml"), "xpecs: []\n").unwrap();
        let add = process::Command::new("git")
            .args(["add", "config.yml"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            add.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr)
        );
        let add_missing_blob = process::Command::new("git")
            .args([
                "update-index",
                "--add",
                "--info-only",
                "--cacheinfo",
                "100644,1111111111111111111111111111111111111111,unrelated.bin",
            ])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            add_missing_blob.status.success(),
            "git update-index failed: {}",
            String::from_utf8_lossy(&add_missing_blob.stderr)
        );

        let content = RepoInspectionCache::new()
            .staged_file_content(&root, "config.yml")
            .unwrap();

        assert_eq!(content, "xpecs: []\n");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: cg,s6
    fn in_place_file_content_reads_listed_symlink() {
        let root = test_root("in-place-file-content-reads-symlink");
        let outside = outside_test_file(&root);
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("config.yml")).unwrap();

        let content = in_place_file_content_from_fs(&root, Path::new("config.yml")).unwrap();

        assert_eq!(content, "secret");
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test] // xpec: I4
    fn in_place_file_listing_includes_symlinks() {
        let root = test_root("in-place-file-listing-includes-symlinks");
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(root.join("specs/real.md"), "real").unwrap();
        let outside = outside_test_file(&root);
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("specs/link.md")).unwrap();

        let files = in_place_file_listing(&root).unwrap();

        assert_eq!(
            files,
            vec![b"specs/link.md".to_vec(), b"specs/real.md".to_vec()]
        );
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: I4
    fn in_place_file_listing_ignores_git_directory_or_gitfile_only() {
        for gitfile in [false, true] {
            let kind = if gitfile { "gitfile" } else { "directory" };
            let root = test_root(&format!("in-place-listing-git-{kind}"));
            if gitfile {
                fs::write(root.join(".git"), "gitdir: ../metadata\n").unwrap();
            } else {
                fs::create_dir_all(root.join(".git/objects")).unwrap();
                fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            }
            fs::write(root.join(".gitignore"), "target/\n").unwrap();
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

            let files = in_place_file_listing(&root).unwrap();

            assert_eq!(
                files,
                vec![b".gitignore".to_vec(), b"src/main.rs".to_vec()],
                "in-place listing must ignore a .git {kind} without hiding project files"
            );
            let _ = fs::remove_dir_all(root);
        }
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
