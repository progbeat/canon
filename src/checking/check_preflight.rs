use crate::platform;
use std::path::Path;
use std::process::Command;

pub(crate) fn install_sigint_handler() -> Result<(), String> {
    platform::install_check_signal_handlers()
}

pub(crate) fn reset_check_interrupted() {
    platform::reset_check_interrupted();
}

pub(crate) fn check_interrupted() -> bool {
    platform::check_interrupted()
}

#[cfg(test)]
pub(crate) fn staged_changed_paths(root: &Path) -> Result<Vec<String>, String> {
    Ok(staged_changed_path_bytes(root)?
        .into_iter()
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .collect())
}

pub(crate) fn staged_changed_path_bytes(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-status")
        .arg("-z")
        // Gate classification needs every staged path, including deletions
        // under `.canon/**`, so do not restrict diff status codes here.
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    staged_changed_path_bytes_from_name_status_z(&output.stdout)
}

#[cfg(test)]
pub(crate) fn staged_changed_paths_from_name_status_z(
    stdout: &[u8],
) -> Result<Vec<String>, String> {
    Ok(staged_changed_path_bytes_from_name_status_z(stdout)?
        .into_iter()
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .collect())
}

pub(crate) fn staged_changed_path_bytes_from_name_status_z(
    stdout: &[u8],
) -> Result<Vec<Vec<u8>>, String> {
    let mut fields = stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else {
            return Err("git diff name-status output ended before path".to_string());
        };
        paths.push(path.to_vec());
        if status.starts_with(b"R") || status.starts_with(b"C") {
            let Some(path) = fields.next() else {
                return Err("git diff name-status output ended before rename/copy path".to_string());
            };
            paths.push(path.to_vec());
        }
    }
    Ok(paths)
}

pub(crate) fn is_canon_project_path_bytes(path: &[u8]) -> bool {
    path.starts_with(b".canon/")
}

pub(crate) fn is_canon_only_staged_change_bytes(paths: &[Vec<u8>]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_canon_project_path_bytes(path))
}
