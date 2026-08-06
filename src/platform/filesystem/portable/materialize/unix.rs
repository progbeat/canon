use super::super::unix::{PlatformError, PlatformResult};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::Path;

pub(in super::super) fn create_materialized_symlink(
    target: &[u8],
    link: &Path,
) -> PlatformResult<()> {
    let target = std::ffi::OsStr::from_bytes(target);
    symlink(target, link).map_err(|err| {
        PlatformError::io(
            format!("failed to symlink evaluator file {}", link.display()),
            err,
        )
    })
}

pub(in super::super) fn hardlink_file_or_copy_symlink(
    source: &Path,
    target: &Path,
) -> PlatformResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|err| {
        PlatformError::io(
            format!("failed to inspect evaluator file {}", source.display()),
            err,
        )
    })?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source).map_err(|err| {
            PlatformError::io(format!("failed to read symlink {}", source.display()), err)
        })?;
        symlink(&link_target, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to copy evaluator symlink {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        })
    } else {
        fs::hard_link(source, target).map_err(|err| {
            PlatformError::io(
                format!(
                    "failed to hardlink evaluator scope file {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        })
    }
}
