use crate::platform::path_from_git_stdout;
use crate::project::command_output_trimmed;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Clone)]
pub(crate) struct StagedTrackedFile {
    pub(crate) path: Vec<u8>,
    pub(crate) mode: String,
    pub(crate) object_id: String,
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
    Ok(root.join(path_from_git_stdout(output.stdout)))
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

pub(crate) fn staged_tracked_files(root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--stage"])
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

#[cfg(all(test, unix))]
pub(crate) fn read_git_blobs_with_git_program(
    root: &Path,
    object_ids: &[String],
    git_program: &OsStr,
) -> Result<Vec<Vec<u8>>, String> {
    read_git_blobs_with_git_program_inner(root, object_ids, git_program)
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
