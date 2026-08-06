use super::*;

#[test] // xpec: t,d
fn staged_file_content_does_not_read_unrelated_blobs() {
    let root = test_root("staged-file-content-requested-blob-only");
    let init = process::Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    fs::write(root.join("config.yml"), "xpecs: []\n").unwrap();
    let add = process::Command::new("git")
        .args(["add", "config.yml"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let add_missing_blob = process::Command::new("git")
        .args([
            "update-index",
            "--add",
            "--info-only",
            "--cacheinfo",
            "100644,1111111111111111111111111111111111111111,unrelated.bin",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        add_missing_blob.status.success(),
        "git update-index failed: {}",
        String::from_utf8_lossy(&add_missing_blob.stderr)
    );

    let content = RepoInspectionCache::new()
        .staged_file_content(&root, "config.yml")
        .unwrap();

    assert_eq!(content, "xpecs: []\n");
    let _ = fs::remove_dir_all(root);
}
