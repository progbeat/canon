use std::fs;
use std::path::Path;

pub(crate) fn validate_snapshot_contains_no_symlinks(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "staged snapshot contains symlink {}; refusing to expose symlinks to evaluator sessions",
            path.display()
        ));
    }
    if file_type.is_dir() {
        for entry in fs::read_dir(path).map_err(|err| {
            format!(
                "failed to read snapshot directory {}: {}",
                path.display(),
                err
            )
        })? {
            let entry = entry.map_err(|err| {
                format!(
                    "failed to read snapshot directory {}: {}",
                    path.display(),
                    err
                )
            })?;
            validate_snapshot_contains_no_symlinks(&entry.path())?;
        }
    }
    Ok(())
}
