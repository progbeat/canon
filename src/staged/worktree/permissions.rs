use crate::platform;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub(super) fn remove_write_permissions_from_materialized_dir(path: &Path) -> Result<(), String> {
    platform::set_materialized_dir_permissions(path)
}

pub(super) fn remove_write_permissions_from_extracted_file(
    path: &Path,
    mode: &str,
) -> Result<(), String> {
    platform::set_materialized_file_permissions(path, mode)
}

pub(super) fn make_materialization_tree_private(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "failed to inspect evaluator materialization directory {}: {}",
                path.display(),
                err
            ));
        }
    };
    if !metadata.file_type().is_dir() {
        return platform::set_private_file_permissions(path);
    }
    platform::set_private_dir_permissions(path)?;
    for entry in fs::read_dir(path).map_err(|err| {
        format!(
            "failed to read evaluator materialization directory {}: {}",
            path.display(),
            err
        )
    })? {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read evaluator materialization directory entry in {}: {}",
                path.display(),
                err
            )
        })?;
        make_materialization_tree_private(&entry.path())?;
    }
    Ok(())
}
