use super::super::git_path::windows::git_bytes_os_string;
use std::fs;
use std::path::{Path, PathBuf};

pub(in super::super) fn create_materialized_symlink(
    target: &[u8],
    link: &Path,
) -> Result<(), String> {
    let target = PathBuf::from(git_bytes_os_string(target.to_vec())?);
    std::os::windows::fs::symlink_file(&target, link).map_err(|err| {
        format!(
            "failed to symlink evaluator file {} to {}: {}",
            link.display(),
            target.display(),
            err
        )
    })
}

pub(in super::super) fn hardlink_file_or_copy_symlink(
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        format!(
            "failed to inspect evaluator file {}: {}",
            source.display(),
            err
        )
    })?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source)
            .map_err(|err| format!("failed to read symlink {}: {}", source.display(), err))?;
        return std::os::windows::fs::symlink_file(&link_target, target).map_err(|err| {
            format!(
                "failed to copy evaluator symlink {} to {}: {}",
                source.display(),
                target.display(),
                err
            )
        });
    }
    fs::hard_link(source, target).map_err(|err| {
        format!(
            "failed to hardlink evaluator scope file {} to {}: {}",
            source.display(),
            target.display(),
            err
        )
    })
}
