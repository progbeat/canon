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
        return Ok(());
    }
    // [tb] Materialized regular files are hardlinks to read-only lazy entries.
    // Re-open only directories for removal; changing a file mode here would
    // also make its cached lazy inode writable.
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: tb
    fn removal_preparation_preserves_read_only_lazy_hardlinks() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-materialization-cleanup-{}-{unique}",
            process::id()
        ));
        let lazy_file = root.join("lazy/file");
        let materialized_root = root.join("tree");
        let materialized_dir = materialized_root.join("dir");
        fs::create_dir_all(lazy_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&materialized_dir).unwrap();
        fs::write(&lazy_file, "content").unwrap();
        fs::set_permissions(&lazy_file, fs::Permissions::from_mode(0o444)).unwrap();
        fs::hard_link(&lazy_file, materialized_dir.join("file")).unwrap();
        fs::set_permissions(&materialized_dir, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&materialized_root, fs::Permissions::from_mode(0o555)).unwrap();

        make_materialization_tree_private(&materialized_root).unwrap();
        fs::remove_dir_all(&materialized_root).unwrap();

        assert_eq!(
            fs::metadata(&lazy_file).unwrap().permissions().mode() & 0o222,
            0
        );

        fs::remove_dir_all(root).unwrap();
    }
}
