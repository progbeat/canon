mod blob_reader;
mod tracked_files;

use crate::output::command_output_trimmed;
use crate::platform::filesystem::path_from_git_stdout;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) use blob_reader::{read_git_blobs, GitBlobReader};
pub(crate) use tracked_files::{
    staged_tracked_files, staged_tracked_files_for_pathspecs, tree_tracked_files_for_pathspecs,
    tree_tracked_files_for_pathspecs_in_environment, TrackedFile,
};

pub(crate) fn git_project_root(start: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to find git project root: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    path_from_git_stdout(output.stdout)
}

pub(crate) fn is_git_worktree(root: &Path) -> Result<bool, String> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to run git rev-parse: {}", err)),
    };
    Ok(output.status.success()
        && command_output_token(&output.stdout, "git rev-parse stdout")? == "true")
}

pub(crate) fn git_common_dir(root: &Path) -> Result<PathBuf, String> {
    resolve_rev_parse_path(root, &["--git-common-dir"])
        .map_err(|err| format!("failed to resolve git common dir: {err}"))
}

pub(crate) fn resolve_git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    resolve_rev_parse_path(root, &["--git-path", path])
}

fn resolve_rev_parse_path(root: &Path, args: &[&str]) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .args(args)
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve Git path: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    let path = path_from_git_stdout(output.stdout)?;
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

pub(super) fn command_output_token<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    let bytes = bytes
        .strip_suffix(b"\r\n")
        .or_else(|| bytes.strip_suffix(b"\n"))
        .ok_or_else(|| format!("{} must contain one line-terminated token", description))?;
    let text = std::str::from_utf8(bytes)
        .map_err(|err| format!("{} must be valid UTF-8: {}", description, err))?;
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return Err(format!("{} must contain one nonempty token", description));
    }
    Ok(text)
}

pub(crate) fn resolve_tree_oid_if_exists(
    root: &Path,
    treeish: &str,
) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "rev-parse",
            "--verify",
            "-q",
            &format!("{treeish}^{{tree}}"),
        ])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if output.status.success() {
        return command_output_token(&output.stdout, "git rev-parse stdout")
            .map(str::to_string)
            .map(Some);
    }
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    Err(format!(
        "not a valid Git tree ({})",
        command_output_trimmed(&output.stderr, "git rev-parse stderr")
            .unwrap_or("git rev-parse failed")
    ))
}

pub(crate) fn staged_tree_oid(root: &Path) -> Result<String, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root);
    write_tree_oid(command)
}

pub(super) fn write_tree_oid(mut command: Command) -> Result<String, String> {
    let output = command
        .arg("write-tree")
        .output()
        .map_err(|err| format!("failed to run git write-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve staged tree: {}",
            command_output_trimmed(&output.stderr, "git write-tree stderr")?
        ));
    }
    command_output_token(&output.stdout, "git write-tree stdout").map(str::to_string)
}

pub(crate) fn compute_empty_tree_oid(root: &Path) -> Result<String, String> {
    // [l] Deliberately omit `-UR`: `hash-object` computes the repository's
    // format-specific empty-tree OID without writing the object to its ODB.
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["hash-object", "-t", "tree", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run git hash-object: {}", err))?;
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .map_err(|err| format!("failed to read git hash-object output: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve empty tree: {}",
            command_output_trimmed(&output.stderr, "git hash-object stderr")?
        ));
    }
    command_output_token(&output.stdout, "git hash-object stdout").map(str::to_string)
}

pub(crate) fn abbreviate_git_oid(root: &Path, oid: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--short", oid])
        .output()
        .map_err(|err| format!("failed to run git rev-parse --short: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to abbreviate Git object {}: {}",
            oid,
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    command_output_token(&output.stdout, "git rev-parse stdout").map(str::to_string)
}

pub(crate) fn tree_object_exists(root: &Path, tree_oid: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-t", tree_oid])
        .output()
        .map_err(|err| format!("failed to run git cat-file: {}", err))?;
    if output.status.success() {
        let object_type = command_output_token(&output.stdout, "git cat-file stdout")?;
        return Ok(object_type == "tree");
    }
    let stderr = command_output_trimmed(&output.stderr, "git cat-file stderr")
        .unwrap_or("git cat-file failed");
    if git_cat_file_reports_missing_object(stderr) {
        return Ok(false);
    }
    Err(format!(
        "failed to inspect Git tree object {}: {}",
        tree_oid, stderr
    ))
}

fn git_cat_file_reports_missing_object(stderr: &str) -> bool {
    stderr.contains("git cat-file: could not get object info")
}

#[cfg(test)]
mod tests {
    use super::{command_output_token, compute_empty_tree_oid, tree_object_exists};
    use crate::output::command_output_trimmed;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: gO
    fn command_output_token_accepts_platform_line_terminators() {
        assert_eq!(
            command_output_token(b"token\n", "test output").unwrap(),
            "token"
        );
        assert_eq!(
            command_output_token(b"token\r\n", "test output").unwrap(),
            "token"
        );
    }

    #[test] // xpec: gO
    fn command_output_token_rejects_changed_or_ambiguous_content() {
        assert!(command_output_token(b"token", "test output").is_err());
        assert!(command_output_token(b"first\nsecond\n", "test output").is_err());
        assert!(command_output_token(b" token \n", "test output").is_err());
    }

    #[test] // xpec: l
    fn computing_empty_tree_oid_does_not_write_to_repository_object_database() {
        let repo = temp_root("compute-empty-tree");
        run_git(&repo, &["init"]).unwrap();
        let objects = repo.join(".git").join("objects");
        let files_before = files_below(&objects);

        let oid = compute_empty_tree_oid(&repo).unwrap();

        assert!(!oid.is_empty());
        assert_eq!(files_below(&objects), files_before);
        fs::remove_dir_all(repo).unwrap();
    }

    #[test] // xpec: gO
    fn tree_object_exists_reports_missing_tree_without_masking_git_failures() {
        let repo = temp_root("tree-object-exists");
        run_git(&repo, &["init"]).unwrap();
        let tree_oid = write_empty_tree(&repo);

        assert!(tree_object_exists(&repo, &tree_oid).unwrap());
        assert!(!tree_object_exists(&repo, "0000000000000000000000000000000000000000").unwrap());

        let not_repo = temp_root("tree-object-not-repo");
        let err = tree_object_exists(&not_repo, &tree_oid).unwrap_err();
        assert!(err.contains("not a git repository"));

        fs::remove_dir_all(repo).unwrap();
        fs::remove_dir_all(not_repo).unwrap();
    }

    fn write_empty_tree(root: &Path) -> String {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["hash-object", "-t", "tree", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(b"").unwrap();
        let output = child.wait_with_output().unwrap();
        // xpec: gO
        assert!(output.status.success());
        command_output_token(&output.stdout, "git hash-object stdout")
            .unwrap()
            .to_string()
    }

    fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        if output.status.success() {
            Ok(())
        } else {
            Err(command_output_trimmed(&output.stderr, "git stderr")
                .unwrap_or("git failed")
                .to_string())
        }
    }

    fn files_below(root: &Path) -> Vec<PathBuf> {
        let mut pending = vec![root.to_path_buf()];
        let mut files = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else {
                    files.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
                }
            }
        }
        files.sort();
        files
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("canon-git-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
