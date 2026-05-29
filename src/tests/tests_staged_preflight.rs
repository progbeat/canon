use super::*;

#[test]
fn staged_changed_paths_include_rename_sources() {
    let root = git_project("rename-into-canon");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::rename(root.join("src/main.rs"), root.join(".canon/main.rs")).unwrap();
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let paths = staged_changed_paths(&root).unwrap();

    assert!(paths.contains(&"src/main.rs".to_string()));
    assert!(paths.contains(&".canon/main.rs".to_string()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_changed_paths_tolerate_non_utf8_paths() {
    let paths = staged_changed_paths_from_name_status_z(b"A\0nonutf8-\xff\0").unwrap();

    assert_eq!(paths.len(), 1);
}

#[test]
fn check_command_logs_start_and_finish_for_config_load_failure() {
    let root = git_project("check-config-load-log");
    enable_diagnostic_logs(&root);
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), "version: 1\nagent: []\n").unwrap();
    let output = Command::new("git")
        .args(["add", CHECK_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_check_command(&root, &[]).unwrap_err();

    assert!(err.to_string().contains("failed to parse .canon/check.yml"));
    let log = fs::read_to_string(root.join(".git/canon/logs/0.jsonl")).unwrap();
    assert!(log.contains(r#""event":"check.start""#));
    assert!(log.contains(r#""event":"check.finish""#));
    assert!(log.contains(r#""query":false"#));
    assert!(log.contains(r#""status":"error""#));
    assert!(!log.contains(r#""errors":"#));
    assert!(log.contains("failed to parse .canon/check.yml"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_loads_staged_default_config_not_worktree() {
    let root = git_project("check-config-staged-default");
    write_check_config(&root);
    let output = Command::new("git")
        .args(["add", CHECK_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join(CHECK_PATH), "version: 1\nagent: []\n").unwrap();
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new(CHECK_PATH), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_loads_explicit_tree_config_not_staged_or_worktree() {
    let root = git_project("check-config-explicit-tree");
    fs::create_dir_all(root.join(".canon/specs")).unwrap();
    fs::write(root.join(CHECK_PATH), tree_check_config("Head?")).unwrap();
    fs::write(root.join(".canon/specs/a.md"), "head spec").unwrap();
    Command::new("git")
        .args(["add", CHECK_PATH, ".canon/specs/a.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "head canon");
    fs::write(root.join(CHECK_PATH), tree_check_config("Staged?")).unwrap();
    fs::write(root.join(".canon/specs/a.md"), "staged spec").unwrap();
    Command::new("git")
        .args(["add", CHECK_PATH, ".canon/specs/a.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join(CHECK_PATH), tree_check_config("Worktree?")).unwrap();
    fs::write(root.join(".canon/specs/a.md"), "worktree spec").unwrap();
    let source = TreeSource::resolve(&root, "HEAD", "--tree").unwrap();
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new(CHECK_PATH), &source)
        .unwrap();

    assert_eq!(config.expectations.len(), 2);
    assert_eq!(config.expectations[0].q, "Head?");
    assert_eq!(config.expectations[1].q, "head spec");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_loads_staged_custom_config_not_worktree() {
    let root = git_project("check-config-staged-custom");
    let alt = "alt-check.yml";
    fs::write(root.join(alt), check_config_yaml()).unwrap();
    let output = Command::new("git")
        .args(["add", alt])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join(alt), "version: 1\nagent: []\n").unwrap();
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new(alt), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_rejects_untracked_default_config() {
    let root = git_project("check-config-untracked-default");
    write_check_config(&root);
    let mut cache = RepoInspectionCache::new();

    let err = cache
        .load_check_config(&root, Path::new(CHECK_PATH), &TreeSource::Staged)
        .unwrap_err();

    assert!(err.contains("No canon check config found at .canon/check.yml"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_rejects_untracked_custom_config() {
    let root = git_project("check-config-untracked-custom");
    commit_all(&root, "initial");
    let alt = "other-check.yml";
    fs::write(root.join(alt), check_config_yaml()).unwrap();
    let mut cache = RepoInspectionCache::new();

    let err = cache
        .load_check_config(&root, Path::new(alt), &TreeSource::Staged)
        .unwrap_err();

    assert!(err.contains("failed to read staged other-check.yml"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_custom_config_ignores_unmatched_includes() {
    let root = git_project("check-config-staged-custom-include");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join("checks/expects")).unwrap();
    fs::write(
        root.join("checks/expects/project.yml"),
        r#"
- q: "Included?"
  a: "yes"
"#,
    )
    .unwrap();
    fs::write(
        root.join("checks/check.yml"),
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - include: "expects/*.yml"
  - q: "Local?"
    a: "no"
"#,
    )
    .unwrap();
    let output = Command::new("git")
        .args(["add", "checks/check.yml"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new("checks/check.yml"), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(config.expectations[0].q, "Local?");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_custom_config_expands_staged_includes() {
    let root = git_project("check-config-staged-custom-include-success");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join("checks/expects")).unwrap();
    fs::write(
        root.join("checks/expects/project.yml"),
        r#"
- q: "Included?"
  a: "yes"
"#,
    )
    .unwrap();
    fs::write(
        root.join("checks/check.yml"),
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - include: "expects/*.yml"
"#,
    )
    .unwrap();
    let output = Command::new("git")
        .args(["add", "checks/check.yml", "checks/expects/project.yml"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new("checks/check.yml"), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(config.expectations[0].q, "Included?");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_custom_config_expands_staged_path_generators() {
    let root = git_project("check-config-staged-custom-path-generator");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join("checks/specs")).unwrap();
    fs::write(root.join("checks/specs/a.md"), "staged spec").unwrap();
    fs::write(
        root.join("checks/check.yml"),
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - path: "specs/*.md"
    q_template: "{{content}}"
    a: "yes"
"#,
    )
    .unwrap();
    let output = Command::new("git")
        .args(["add", "checks/check.yml", "checks/specs/a.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join("checks/specs/a.md"), "worktree spec").unwrap();
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new("checks/check.yml"), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(config.expectations[0].q, "staged spec");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_staged_delete_does_not_fall_back_to_worktree() {
    let root = git_project("check-config-staged-delete");
    write_check_config(&root);
    Command::new("git")
        .args(["add", CHECK_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let output = Command::new("git")
        .args(["rm", "--cached", CHECK_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut cache = RepoInspectionCache::new();

    let err = cache
        .load_check_config(&root, Path::new(CHECK_PATH), &TreeSource::Staged)
        .unwrap_err();

    assert!(err.contains("No canon check config found at .canon/check.yml"));
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn check_config_literal_pathspec_name_loads_staged_content() {
    let root = git_project("check-config-literal-pathspec");
    let path = ":(literal)check.yml";
    fs::write(root.join(path), check_config_yaml()).unwrap();
    let output = Command::new("git")
        .args(["--literal-pathspecs", "add", "--", path])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join(path), "version: 1\nagent: []\n").unwrap();
    let mut cache = RepoInspectionCache::new();

    let config = cache
        .load_check_config(&root, Path::new(path), &TreeSource::Staged)
        .unwrap();

    assert_eq!(config.expectations.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_command_logs_start_and_finish_for_cache_cleanup_failure() {
    let root = git_project("check-cache-cleanup-log");
    enable_diagnostic_logs(&root);
    commit_all(&root, "initial");
    write_check_config(&root);
    let output = Command::new("git")
        .args(["add", CHECK_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cache_path = root.join(".git/canon/cache");
    ensure_dir(cache_path.parent().unwrap()).unwrap();
    fs::write(&cache_path, "not a directory").unwrap();

    let err = run_check_command(&root, &[]).unwrap_err();

    assert!(!err.to_string().is_empty());
    let log = fs::read_to_string(root.join(".git/canon/logs/0.jsonl")).unwrap();
    assert!(log.contains(r#""event":"check.start""#));
    assert!(log.contains(r#""event":"check.finish""#));
    assert!(!log.contains(r#""errors":"#));
    assert!(log.contains("failed to read"));
    let _ = fs::remove_dir_all(root);
}

fn tree_check_config(question: &str) -> String {
    format!(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "{question}"
    a: "yes"
  - path: "specs/*.md"
    q_template: "{{{{content}}}}"
    a: "yes"
"#
    )
}
