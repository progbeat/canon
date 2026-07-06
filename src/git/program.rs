use crate::platform::path_from_git_stdout;
use crate::project::command_output_trimmed;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Clone)]
pub(crate) struct StagedTrackedFile {
    pub(crate) path: Vec<u8>,
    pub(crate) mode: String,
    pub(crate) object_id: String,
}

impl StagedTrackedFile {
    pub(crate) fn is_blob_file_entry(&self) -> bool {
        // Canon's lazy materialization policy calls `read_blob` for each
        // file_entries(git_tree) item. Regular files, executable files, and
        // Git symlink entries are blob-backed file entries; gitlinks point at
        // commit objects, so they are not file entries for this policy.
        matches!(self.mode.as_str(), "100644" | "100755" | "120000")
    }
}

pub(crate) fn resolve_git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--git-path")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve git path {}: {}",
            path,
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    Ok(root.join(path_from_git_stdout(output.stdout)?))
}

pub(crate) fn git_head_tree_exists(root: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", "HEAD^{tree}"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    Ok(output.status.success())
}

pub(crate) fn staged_tree_oid(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("write-tree")
        .output()
        .map_err(|err| format!("failed to run git write-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve staged tree: {}",
            command_output_trimmed(&output.stderr, "git write-tree stderr")?
        ));
    }
    command_output_trimmed(&output.stdout, "git write-tree stdout").map(str::to_string)
}

pub(crate) fn empty_tree_oid(root: &Path) -> Result<String, String> {
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
    command_output_trimmed(&output.stdout, "git hash-object stdout").map(str::to_string)
}

pub(crate) fn staged_tracked_files(root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
    tracked_files_for_pathspecs(root, None, &[])
}

pub(crate) fn resolve_tree_oid(root: &Path, treeish: &str) -> Result<String, String> {
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
    if !output.status.success() {
        return Err(format!(
            "not a valid Git tree ({})",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")
                .unwrap_or("git rev-parse failed")
        ));
    }
    command_output_trimmed(&output.stdout, "git rev-parse stdout").map(str::to_string)
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
    command_output_trimmed(&output.stdout, "git rev-parse stdout").map(str::to_string)
}

pub(crate) fn tree_object_exists(root: &Path, tree_oid: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", &format!("{tree_oid}^{{tree}}")])
        .output()
        .map_err(|err| format!("failed to run git cat-file: {}", err))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = command_output_trimmed(&output.stderr, "git cat-file stderr")
        .unwrap_or("git cat-file failed");
    if git_cat_file_reports_unusable_tree_object(stderr, tree_oid) {
        return Ok(false);
    }
    Err(format!(
        "failed to inspect Git tree object {}: {}",
        tree_oid, stderr
    ))
}

fn git_cat_file_reports_unusable_tree_object(stderr: &str, tree_oid: &str) -> bool {
    let tree_name = format!("{tree_oid}^{{tree}}");
    stderr.contains(&format!("Not a valid object name {tree_name}"))
        || stderr.contains(&format!(
            "{tree_name}: expected tree type, but the object dereferences to"
        ))
}

pub(crate) fn tree_tracked_files(
    root: &Path,
    treeish: &str,
) -> Result<Vec<StagedTrackedFile>, String> {
    tree_tracked_files_for_pathspecs(root, treeish, &[])
}

pub(super) fn tree_tracked_files_for_pathspecs(
    root: &Path,
    treeish: &str,
    pathspecs: &[String],
) -> Result<Vec<StagedTrackedFile>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-tree", "-rz", "--full-tree", "-r", treeish, "--"])
        .args(pathspecs)
        .output()
        .map_err(|err| format!("failed to run git ls-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect Git tree {}: {}",
            treeish,
            command_output_trimmed(&output.stderr, "git ls-tree stderr")?
        ));
    }
    parse_tree_tracked_files(&output.stdout)
}

fn tracked_files_for_pathspecs(
    root: &Path,
    index_file: Option<&Path>,
    pathspecs: &[String],
) -> Result<Vec<StagedTrackedFile>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--stage", "--"])
        .args(pathspecs);
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git ls-files: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect staged files: {}",
            command_output_trimmed(&output.stderr, "git ls-files stderr")?
        ));
    }
    let mut files = Vec::new();
    for entry in output.stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        if let Some(file) = parse_staged_tracked_file(entry)? {
            files.push(file);
        }
    }
    Ok(files)
}

fn parse_tree_tracked_files(stdout: &[u8]) -> Result<Vec<StagedTrackedFile>, String> {
    let mut files = Vec::new();
    for entry in stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        files.push(parse_tree_tracked_file(entry)?);
    }
    Ok(files)
}

fn parse_tree_tracked_file(entry: &[u8]) -> Result<StagedTrackedFile, String> {
    let tab = entry
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| "git ls-tree entry missing path separator".to_string())?;
    let metadata = std::str::from_utf8(&entry[..tab])
        .map_err(|_| "git ls-tree entry metadata must be valid UTF-8".to_string())?;
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| "git ls-tree entry missing mode".to_string())?;
    let _kind = fields
        .next()
        .ok_or_else(|| "git ls-tree entry missing object type".to_string())?;
    let object_id = fields
        .next()
        .ok_or_else(|| "git ls-tree entry missing object id".to_string())?;
    Ok(StagedTrackedFile {
        path: entry[tab + 1..].to_vec(),
        mode: mode.to_string(),
        object_id: object_id.to_string(),
    })
}

fn parse_staged_tracked_file(entry: &[u8]) -> Result<Option<StagedTrackedFile>, String> {
    let tab = entry
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| "git ls-files entry missing path separator".to_string())?;
    let metadata = std::str::from_utf8(&entry[..tab])
        .map_err(|_| "git ls-files entry metadata must be valid UTF-8".to_string())?;
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| "git ls-files entry missing mode".to_string())?;
    let object_id = fields
        .next()
        .ok_or_else(|| "git ls-files entry missing object id".to_string())?;
    let stage = fields
        .next()
        .ok_or_else(|| "git ls-files entry missing stage".to_string())?;
    if stage != "0" {
        return Ok(None);
    }
    Ok(Some(StagedTrackedFile {
        path: entry[tab + 1..].to_vec(),
        mode: mode.to_string(),
        object_id: object_id.to_string(),
    }))
}

pub(crate) fn read_git_blobs(root: &Path, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
    read_git_blobs_with_git_program_inner(root, object_ids, OsStr::new("git"))
}

pub(crate) struct GitBlobReader {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl GitBlobReader {
    pub(crate) fn new(root: &Path) -> Result<GitBlobReader, String> {
        GitBlobReader::new_with_git_program(root, OsStr::new("git"))
    }

    fn new_with_git_program(root: &Path, git_program: &OsStr) -> Result<GitBlobReader, String> {
        let mut child = Command::new(git_program)
            .arg("-C")
            .arg(root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to run git cat-file: {}", err))?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                return Err(cleanup_git_cat_file_child(
                    child,
                    "failed to open git cat-file stdin".to_string(),
                ))
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                drop(stdin);
                return Err(cleanup_git_cat_file_child(
                    child,
                    "failed to open git cat-file stdout".to_string(),
                ));
            }
        };
        Ok(GitBlobReader {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub(crate) fn read_blobs(&mut self, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
        let mut blobs = Vec::with_capacity(object_ids.len());
        for object_id in object_ids {
            blobs.push(self.read_blob(object_id)?);
        }
        Ok(blobs)
    }

    fn read_blob(&mut self, object_id: &str) -> Result<Vec<u8>, String> {
        writeln!(self.stdin, "{}", object_id)
            .map_err(|err| format!("failed to write git cat-file input: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to write git cat-file input: {}", err))?;

        let mut header = String::new();
        let bytes_read = self
            .stdout
            .read_line(&mut header)
            .map_err(|err| format!("failed to read git cat-file output: {}", err))?;
        if bytes_read == 0 {
            return Err(format!(
                "git cat-file output missing header for {}",
                object_id
            ));
        }
        let header = header.trim_end_matches('\n');
        let mut fields = header.split_whitespace();
        let actual_id = fields
            .next()
            .ok_or_else(|| "git cat-file header missing object id".to_string())?;
        let object_type = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing type for {}", actual_id))?;
        if object_type == "missing" {
            return Err(format!("staged blob {} is missing", actual_id));
        }
        if object_type != "blob" {
            return Err(format!(
                "staged object {} is {}, not blob",
                actual_id, object_type
            ));
        }
        let size = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing size for {}", actual_id))?
            .parse::<usize>()
            .map_err(|_| format!("git cat-file header has invalid size for {}", actual_id))?;
        let mut blob = vec![0; size];
        self.stdout
            .read_exact(&mut blob)
            .map_err(|_| format!("git cat-file output truncated for {}", actual_id))?;
        let mut delimiter = [0u8; 1];
        self.stdout.read_exact(&mut delimiter).map_err(|_| {
            format!(
                "git cat-file output missing object delimiter for {}",
                actual_id
            )
        })?;
        if delimiter != [b'\n'] {
            return Err(format!(
                "git cat-file output missing object delimiter for {}",
                actual_id
            ));
        }
        Ok(blob)
    }
}

impl Drop for GitBlobReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_git_blobs_with_git_program_inner(
    root: &Path,
    object_ids: &[String],
    git_program: &OsStr,
) -> Result<Vec<Vec<u8>>, String> {
    if object_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new(git_program)
        .arg("-C")
        .arg(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run git cat-file: {}", err))?;
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            return Err(cleanup_git_cat_file_child(
                child,
                "failed to open git cat-file stdin".to_string(),
            ))
        }
    };
    for object_id in object_ids {
        if let Err(err) = writeln!(stdin, "{}", object_id) {
            drop(stdin);
            return Err(cleanup_git_cat_file_child(
                child,
                format!("failed to write git cat-file input: {}", err),
            ));
        }
    }
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|err| format!("failed to read git cat-file output: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to read staged blobs: {}",
            command_output_trimmed(&output.stderr, "git cat-file stderr")?
        ));
    }
    parse_git_blob_batch(&output.stdout, object_ids)
}

fn cleanup_git_cat_file_child(child: Child, message: String) -> String {
    match child.wait_with_output() {
        Ok(_) => message,
        Err(err) => format!("{}; failed to reap git cat-file: {}", message, err),
    }
}

fn parse_git_blob_batch(output: &[u8], object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
    let mut offset = 0usize;
    let mut blobs = Vec::with_capacity(object_ids.len());
    for object_id in object_ids {
        let header_end = output[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|relative| offset + relative)
            .ok_or_else(|| format!("git cat-file output missing header for {}", object_id))?;
        let header = std::str::from_utf8(&output[offset..header_end])
            .map_err(|_| "git cat-file header must be valid UTF-8".to_string())?;
        let mut fields = header.split_whitespace();
        let actual_id = fields
            .next()
            .ok_or_else(|| "git cat-file header missing object id".to_string())?;
        let object_type = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing type for {}", actual_id))?;
        if object_type == "missing" {
            return Err(format!("staged blob {} is missing", actual_id));
        }
        if object_type != "blob" {
            return Err(format!(
                "staged object {} is {}, not blob",
                actual_id, object_type
            ));
        }
        let size = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing size for {}", actual_id))?
            .parse::<usize>()
            .map_err(|_| format!("git cat-file header has invalid size for {}", actual_id))?;
        offset = header_end + 1;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| "git cat-file object size overflowed".to_string())?;
        if output.len() < end {
            return Err(format!("git cat-file output truncated for {}", actual_id));
        }
        blobs.push(output[offset..end].to_vec());
        offset = end;
        if output.get(offset) != Some(&b'\n') {
            return Err(format!(
                "git cat-file output missing object delimiter for {}",
                actual_id
            ));
        }
        offset += 1;
    }
    if offset != output.len() {
        return Err("git cat-file output has trailing data".to_string());
    }
    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use super::tree_object_exists;
    use crate::project::command_output_trimmed;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
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
        assert!(output.status.success());
        command_output_trimmed(&output.stdout, "git hash-object stdout")
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
