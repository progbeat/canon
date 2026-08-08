use super::*;

#[cfg(unix)]
#[test] // xpec: 90,jM
fn in_place_file_content_reads_listed_symlink() {
    let root = test_root("in-place-file-content-reads-symlink");
    let outside = outside_test_file(&root);
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("config.yml")).unwrap();

    let content = in_place_file_content_from_fs(&root, Path::new("config.yml")).unwrap();

    assert_eq!(content, "secret");
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test] // xpec: 90
fn in_place_file_listing_includes_symlinks() {
    let root = test_root("in-place-file-listing-includes-symlinks");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/real.md"), "real").unwrap();
    let outside = outside_test_file(&root);
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("specs/link.md")).unwrap();

    let files = in_place_file_listing_from_fs(&root).unwrap();

    assert_eq!(
        files,
        vec![b"specs/link.md".to_vec(), b"specs/real.md".to_vec()]
    );
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 90
fn in_place_file_listing_ignores_git_directory_or_gitfile_only() {
    for gitfile in [false, true] {
        let kind = if gitfile { "gitfile" } else { "directory" };
        let root = test_root(&format!("in-place-listing-git-{kind}"));
        if gitfile {
            fs::write(root.join(".git"), "gitdir: ../metadata\n").unwrap();
        } else {
            fs::create_dir_all(root.join(".git/objects")).unwrap();
            fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let files = in_place_file_listing_from_fs(&root).unwrap();

        assert_eq!(
            files,
            vec![b".gitignore".to_vec(), b"src/main.rs".to_vec()],
            "in-place listing must ignore a .git {kind} without hiding project files"
        );
        let _ = fs::remove_dir_all(root);
    }
}
