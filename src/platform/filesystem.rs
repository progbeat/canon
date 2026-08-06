mod portable;

pub(crate) use portable::{
    chmod_secret_dir_no_access, create_materialized_symlink, create_private_dir,
    create_private_dir_all, git_path_bytes, hardlink_file_or_copy_symlink,
    make_directory_tree_private, make_hook_executable, mirror_evaluator_codex_home_file, move_path,
    os_string_from_bytes, path_from_git_stdout, restore_secret_dir_mode, secret_dir_mode,
    set_materialized_dir_permissions, set_materialized_file_permissions,
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator, SecretDirMode,
};

#[cfg(all(test, unix))]
mod tests {
    use super::make_directory_tree_private;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: 1t
    fn making_a_directory_tree_private_leaves_regular_files_read_only() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!(
                "canon-directory-tree-permissions-{}-{unique}",
                std::process::id()
            ));
        let directory = root.join("directory");
        let file = directory.join("file");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&file, "content").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();

        make_directory_tree_private(&root).unwrap();

        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        fs::remove_dir_all(root).unwrap();
    }
}
