use crate::platform::filesystem::git_path_bytes;
use std::fs;
use std::path::Path;

pub(super) fn file_content(root: &Path, path: &Path) -> Result<String, String> {
    let path = root.join(path);
    // [90,jM] In-place uses ordinary filesystem semantics. A path discovered
    // from this same source remains readable when it is a symlink.
    fs::read_to_string(&path).map_err(|err| format!("failed to read {}: {}", path.display(), err))
}

pub(super) fn file_listing(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<Vec<u8>>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {}", dir.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        // [90] This listing is used only to discover filesystem inputs for
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
            collect_files(root, &path, files)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| format!("failed to relativize {}", path.display()))?;
            files.push(git_path_bytes(relative)?);
        }
    }
    Ok(())
}
