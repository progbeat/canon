//! Read-only Git-tree materialization.

mod entry_path;
mod extract;
mod hardlink;
mod permissions;
mod root;
mod root_lock;
mod setup;
mod visible_tree;

use crate::git::{GitBlobReader, TreeSource};
use crate::repo_inspection::RepoInspectionCache;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) struct TreeMaterializer {
    source_root: PathBuf,
    source: TreeSource,
    repo_inspection: RefCell<RepoInspectionCache>,
    materialized_input: MaterializedProjectInput,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

/// Filesystem-shaped checked project input exposed to evaluator processes.
///
/// This is project content, not invocation state. When the root belongs to the
/// caller, the in-memory journal is used only to restore that content on drop.
struct MaterializedProjectInput {
    extracted_files_dir: PathBuf,
    visible_trees_dir: PathBuf,
    rollback_journal: Option<RefCell<MaterializationRollbackJournal>>,
    root: root::MaterializedProjectInputRoot,
}

struct MaterializationRollbackJournal {
    extracted_files_dir_created: bool,
    visible_trees_dir_created: bool,
    extracted_path_changes: Vec<RollbackExtractedPathChange>,
    created_visible_tree_roots: BTreeSet<PathBuf>,
}

struct RollbackExtractedPathChange {
    target: PathBuf,
    backup: Option<PathBuf>,
}

impl Drop for MaterializedProjectInput {
    fn drop(&mut self) {
        if self.root.is_canon_owned() {
            let _ = permissions::make_materialization_tree_private(&self.visible_trees_dir);
            return;
        }
        let Some(journal) = self.rollback_journal.as_mut() else {
            return;
        };
        let journal = journal.get_mut();
        for tree_root in &journal.created_visible_tree_roots {
            let _ = remove_materialization_path(tree_root);
        }
        for change in journal.extracted_path_changes.iter().rev() {
            let _ = remove_materialization_path(&change.target);
            if let Some(backup) = &change.backup {
                let _ = std::fs::rename(backup, &change.target);
            }
            remove_empty_extracted_parent_dirs(&change.target, &self.extracted_files_dir);
        }
        if journal.visible_trees_dir_created {
            let _ = std::fs::remove_dir(&self.visible_trees_dir);
        }
        if journal.extracted_files_dir_created {
            let _ = std::fs::remove_dir(&self.extracted_files_dir);
        }
    }
}

fn remove_materialization_path(path: &std::path::Path) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if metadata.file_type().is_dir() {
        permissions::make_materialization_tree_private(path).map_err(std::io::Error::other)?;
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn remove_empty_extracted_parent_dirs(
    path: &std::path::Path,
    extracted_files_dir: &std::path::Path,
) {
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == extracted_files_dir {
            break;
        }
        if std::fs::remove_dir(directory).is_err() {
            break;
        }
        parent = directory.parent();
    }
}

#[cfg(test)]
mod test_support {
    use crate::config_types::AgentConfig;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(super) fn empty_test_agent() -> AgentConfig {
        AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: None,
            ignore_configured: false,
            plugins: Vec::new(),
        }
    }

    pub(super) fn assert_read_only(path: impl AsRef<std::path::Path>) {
        let path = path.as_ref();
        // xpec: 1t
        assert!(
            fs::metadata(path).unwrap().permissions().readonly(),
            "{} should be read-only",
            path.display()
        );
    }

    pub(super) fn git_project(name: &str) -> PathBuf {
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
        Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        for args in [
            ["config", "core.autocrlf", "false"],
            ["config", "core.eol", "lf"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            // xpec: 1t
            assert!(
                output.status.success(),
                "git config failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fs::write(root.join("README.md"), "hello").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        Command::new("git")
            .arg("add")
            .arg(".")
            .current_dir(&root)
            .output()
            .unwrap();
        root
    }
}
