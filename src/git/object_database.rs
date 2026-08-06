use super::program::write_tree_oid;
use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const GIT_PROMPT_OBJECT_ARTIFACT_DIR_PREFIX: &str = "canon-prompt-git-objects";

/// Filesystem-shaped, derived input for Git commands rendered into evaluator prompts.
///
/// Canon's mutable invocation state remains in memory. Git cannot consume an
/// in-memory object map through its normal CLI, so a temporary alternate ODB
/// materializes only the otherwise-missing staged tree object. Like oversized
/// prompt-command output, this is an evaluator input artifact: it prefers a
/// memory-backed temporary parent, falls back according to the common
/// temporary-directory policy, never enters repository state, and is removed
/// with its owner.
pub(crate) struct GitPromptObjectArtifacts {
    _owner: OwnedPrivateTemporaryDirectory,
    object_directory: PathBuf,
    alternate_object_directory: PathBuf,
}

impl GitPromptObjectArtifacts {
    pub(crate) fn new(
        root: &Path,
        temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
    ) -> Result<GitPromptObjectArtifacts, String> {
        let owner = OwnedPrivateTemporaryDirectory::create(
            temporary_directory_allocator,
            GIT_PROMPT_OBJECT_ARTIFACT_DIR_PREFIX,
        )?;
        let object_directory = owner.path().join("objects");
        crate::platform::filesystem::create_private_dir(&object_directory).map_err(|err| {
            format!(
                "failed to create temporary Git prompt object directory {}: {}",
                object_directory.display(),
                err
            )
        })?;
        let alternate_object_directory = repository_object_directory(root)?;
        Ok(GitPromptObjectArtifacts {
            _owner: owner,
            object_directory,
            alternate_object_directory,
        })
    }

    pub(crate) fn staged_tree_oid(&self, root: &Path) -> Result<String, String> {
        write_tree_oid(self.git_command(root))
    }

    pub(crate) fn prompt_environment(&self) -> Vec<(OsString, OsString)> {
        vec![
            (
                OsString::from("GIT_OBJECT_DIRECTORY"),
                self.object_directory.as_os_str().to_os_string(),
            ),
            (
                OsString::from("GIT_ALTERNATE_OBJECT_DIRECTORIES"),
                self.alternate_object_directory.as_os_str().to_os_string(),
            ),
        ]
    }

    fn git_command(&self, root: &Path) -> Command {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(root)
            .env("GIT_OBJECT_DIRECTORY", &self.object_directory)
            .env(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                &self.alternate_object_directory,
            );
        command
    }
}

fn repository_object_directory(root: &Path) -> Result<PathBuf, String> {
    let path = super::program::resolve_git_path(root, "objects")
        .map_err(|err| format!("failed to locate Git object directory: {err}"))?;
    fs::canonicalize(&path).map_err(|err| {
        format!(
            "failed to resolve Git object directory {}: {}",
            path.display(),
            err
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: g2,l,Mo
    fn staged_tree_prompt_artifact_never_enters_repository_object_database() {
        let root = std::env::temp_dir().join(format!(
            "canon-prompt-git-object-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "--quiet"]);
        fs::write(root.join("new.txt"), "new staged contents\n").unwrap();
        run_git(&root, &["add", "new.txt"]);
        let artifacts =
            GitPromptObjectArtifacts::new(&root, &PrivateTemporaryDirectoryAllocator::new())
                .unwrap();

        let tree_oid = artifacts.staged_tree_oid(&root).unwrap();

        assert!(!git_object_exists(&root, &tree_oid, &[]));
        assert!(git_object_exists(
            &root,
            &tree_oid,
            &artifacts.prompt_environment()
        ));
        drop(artifacts);
        assert!(!git_object_exists(&root, &tree_oid, &[]));
        let _ = fs::remove_dir_all(root);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_object_exists(
        root: &Path,
        object_id: &str,
        environment: &[(OsString, OsString)],
    ) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "-e", object_id])
            .envs(environment.iter().map(|(key, value)| (key, value)))
            .status()
            .unwrap()
            .success()
    }
}
