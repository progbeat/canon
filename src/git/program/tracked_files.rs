use crate::output::command_output_trimmed;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

#[derive(Clone)]
pub(crate) struct TrackedFile {
    pub(crate) path: Vec<u8>,
    pub(crate) mode: String,
    pub(crate) object_id: String,
}

impl TrackedFile {
    pub(crate) fn is_blob_file_entry(&self) -> bool {
        // Canon's lazy materialization policy requests blob bytes for each
        // file_entries(git_tree) item. Regular files, executable files, and Git
        // symlink entries are blob-backed file entries; gitlinks point at commit
        // objects, so they are not file entries for this policy.
        matches!(self.mode.as_str(), "100644" | "100755" | "120000")
    }
}

pub(crate) fn staged_tracked_files(root: &Path) -> Result<Vec<TrackedFile>, String> {
    tracked_files_for_pathspecs(root, None, &[])
}

pub(crate) fn staged_tracked_files_for_pathspecs(
    root: &Path,
    pathspecs: &[String],
) -> Result<Vec<TrackedFile>, String> {
    tracked_files_for_pathspecs(root, None, pathspecs)
}

pub(crate) fn tree_tracked_files_for_pathspecs(
    root: &Path,
    treeish: &str,
    pathspecs: &[String],
) -> Result<Vec<TrackedFile>, String> {
    tree_tracked_files_for_pathspecs_in_environment(root, treeish, pathspecs, &[])
}

pub(crate) fn tree_tracked_files_for_pathspecs_in_environment(
    root: &Path,
    treeish: &str,
    pathspecs: &[String],
    environment: &[(OsString, OsString)],
) -> Result<Vec<TrackedFile>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["ls-tree", "-rz", "--full-tree", "-r", treeish, "--"])
        .args(pathspecs)
        .envs(environment.iter().map(|(key, value)| (key, value)));
    let output = command
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
) -> Result<Vec<TrackedFile>, String> {
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
        if let Some(file) = parse_tracked_file_entry(entry, TrackedFileEntryFormat::Index)? {
            files.push(file);
        }
    }
    Ok(files)
}

fn parse_tree_tracked_files(stdout: &[u8]) -> Result<Vec<TrackedFile>, String> {
    let mut files = Vec::new();
    for entry in stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let file = parse_tracked_file_entry(entry, TrackedFileEntryFormat::Tree)?
            .ok_or_else(|| "git ls-tree entry unexpectedly has a nonzero stage".to_string())?;
        files.push(file);
    }
    Ok(files)
}

#[derive(Clone, Copy)]
enum TrackedFileEntryFormat {
    Tree,
    Index,
}

impl TrackedFileEntryFormat {
    fn command(self) -> &'static str {
        match self {
            TrackedFileEntryFormat::Tree => "git ls-tree",
            TrackedFileEntryFormat::Index => "git ls-files",
        }
    }
}

fn parse_tracked_file_entry(
    entry: &[u8],
    format: TrackedFileEntryFormat,
) -> Result<Option<TrackedFile>, String> {
    let command = format.command();
    let tab = entry
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| format!("{command} entry missing path separator"))?;
    let metadata = std::str::from_utf8(&entry[..tab])
        .map_err(|_| format!("{command} entry metadata must be valid UTF-8"))?;
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| format!("{command} entry missing mode"))?;
    let object_id = match format {
        TrackedFileEntryFormat::Tree => {
            fields
                .next()
                .ok_or_else(|| "git ls-tree entry missing object type".to_string())?;
            fields
                .next()
                .ok_or_else(|| "git ls-tree entry missing object id".to_string())?
        }
        TrackedFileEntryFormat::Index => {
            let object_id = fields
                .next()
                .ok_or_else(|| "git ls-files entry missing object id".to_string())?;
            let stage = fields
                .next()
                .ok_or_else(|| "git ls-files entry missing stage".to_string())?;
            if stage != "0" {
                return Ok(None);
            }
            object_id
        }
    };
    Ok(Some(TrackedFile {
        path: entry[tab + 1..].to_vec(),
        mode: mode.to_string(),
        object_id: object_id.to_string(),
    }))
}
