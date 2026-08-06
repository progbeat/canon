use super::*;
use crate::git::TreeSource;

fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|err| format!("failed to run git {}: {err}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn commit_file(root: &Path, content: &str, message: &str) -> Result<(), String> {
    fs::write(root.join("file.txt"), content)
        .map_err(|err| format!("failed to write test file: {err}"))?;
    run_git(root, &["add", "file.txt"])?;
    run_git(root, &["commit", "--quiet", "-m", message])
}

#[test] // xpec: d
fn cloned_cache_reuses_one_git_inspection_snapshot() -> Result<(), String> {
    let root = test_root("shared-git-inspection-cache");
    run_git(&root, &["init", "--quiet"])?;
    fs::write(root.join("file.txt"), "first\n")
        .map_err(|err| format!("failed to write test file: {err}"))?;
    run_git(&root, &["add", "file.txt"])?;
    let mut cache = RepoInspectionCache::new();
    let first = cache.git_tracked_files(&root, &TreeSource::Staged)?;

    // The later index mutation belongs to a future high-level operation.
    // A clone must retain this operation's already observed snapshot.
    fs::write(root.join("file.txt"), "second\n")
        .map_err(|err| format!("failed to write test file: {err}"))?;
    run_git(&root, &["add", "file.txt"])?;
    let changed = RepoInspectionCache::new().git_tracked_files(&root, &TreeSource::Staged)?;
    let shared = cache
        .clone()
        .git_tracked_files(&root, &TreeSource::Staged)?;

    assert_ne!(first[0].object_id, changed[0].object_id);
    assert_eq!(first[0].object_id, shared[0].object_id);
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test] // xpec: d,Tv
fn staged_tree_resolves_to_one_oid_backed_snapshot() -> Result<(), String> {
    let root = test_root("staged-oid-backed-snapshot");
    run_git(&root, &["init", "--quiet"])?;
    fs::write(root.join("file.txt"), "first\n")
        .map_err(|err| format!("failed to write test file: {err}"))?;
    run_git(&root, &["add", "file.txt"])?;
    let mut cache = RepoInspectionCache::new();

    let source = cache.resolve_tree_to_oid_source(&root, ":staged", "--tree")?;
    fs::write(root.join("file.txt"), "second\n")
        .map_err(|err| format!("failed to write test file: {err}"))?;
    run_git(&root, &["add", "file.txt"])?;

    assert_eq!(
        cache.tree_file_content(&root, &source, "file.txt")?,
        "first\n"
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[test] // xpec: d,Tv,w
fn symbolic_tree_resolution_is_shared_across_option_labels() -> Result<(), String> {
    let root = test_root("shared-symbolic-tree-resolution");
    for args in [
        &["init", "--quiet"][..],
        &["config", "user.name", "Canon Test"][..],
        &["config", "user.email", "canon@example.invalid"][..],
    ] {
        run_git(&root, args)?;
    }
    commit_file(&root, "first\n", "first")?;

    let mut cache = RepoInspectionCache::new();
    let tree_first = cache.resolve_tree(&root, "HEAD", "--tree")?;

    commit_file(&root, "second\n", "second")?;
    let against_after_tree = cache.resolve_default_against_tree(&root, "HEAD")?;
    let current_after_tree = RepoInspectionCache::new().resolve_tree(&root, "HEAD", "--tree")?;

    assert_eq!(
        cache.tree_file_content(&root, &tree_first, "file.txt")?,
        "first\n"
    );
    assert_eq!(
        cache.tree_file_content(&root, &against_after_tree, "file.txt")?,
        "first\n"
    );
    assert_eq!(
        cache.tree_file_content(&root, &current_after_tree, "file.txt")?,
        "second\n"
    );

    let mut reverse_cache = RepoInspectionCache::new();
    let against_first = reverse_cache.resolve_default_against_tree(&root, "HEAD")?;

    commit_file(&root, "third\n", "third")?;
    let tree_after_against = reverse_cache.resolve_tree(&root, "HEAD", "--tree")?;
    let current_after_against = RepoInspectionCache::new().resolve_tree(&root, "HEAD", "--tree")?;

    assert_eq!(
        reverse_cache.tree_file_content(&root, &against_first, "file.txt")?,
        "second\n"
    );
    assert_eq!(
        reverse_cache.tree_file_content(&root, &tree_after_against, "file.txt")?,
        "second\n"
    );
    assert_eq!(
        reverse_cache.tree_file_content(&root, &current_after_against, "file.txt")?,
        "third\n"
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}
