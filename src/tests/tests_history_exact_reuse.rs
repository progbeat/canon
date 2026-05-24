use super::*;

#[test]
fn reusable_history_record_uses_current_expectation_metadata() {
    let root = git_project("history-current-number");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: 99,
            result: CheckResult::Pass,
            prompt: Some("old prompt text".to_string()),
            expected: Some("old expected".to_string()),
            observed: "yes".to_string(),
            evidence: "cached answer".to_string(),
            scope: full_scope(),
            scope_hash: staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();

    let mut moved = expectation.clone();
    moved.number = 7;
    let record = reusable_history_record(&root, &config.agent, &moved)
        .unwrap()
        .unwrap();
    assert_eq!(record.number, 7);
    assert_eq!(record.prompt.as_deref(), Some(expectation.q.as_str()));
    assert_eq!(record.expected.as_deref(), Some(expectation.a.as_str()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_cache_is_refreshed_after_append() {
    let root = git_project("history-reuse-cache-append");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true)
        .selected
        .remove(0);
    let mut history_cache = HistoryCache::new();
    let mut scope_hash_cache = ScopeHashCache::new();

    assert!(reusable_history_record_with_cache(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        &mut scope_hash_cache,
    )
    .unwrap()
    .is_none());

    let record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record_with_cache(&root, &expectation, &record, &mut history_cache).unwrap();

    assert!(reusable_history_record_with_cache(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        &mut scope_hash_cache,
    )
    .unwrap()
    .unwrap()
    .passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_with_cache_rechecks_current_scope_hash() {
    let root = git_project("history-reuse-cache-rechecks-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true)
        .selected
        .remove(0);
    let record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record(&root, &expectation, &record).unwrap();
    let mut history_cache = HistoryCache::new();
    let mut scope_hash_cache = ScopeHashCache::new();

    assert!(reusable_history_record_with_cache(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        &mut scope_hash_cache,
    )
    .unwrap()
    .is_some());

    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let mut fresh_scope_hash_cache = ScopeHashCache::new();

    assert!(reusable_history_record_with_cache(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        &mut fresh_scope_hash_cache,
    )
    .unwrap()
    .is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_skips_non_answer_history_before_scope_hash_match() {
    let root = git_project("history-reuse-skip-non-answer-before-hash");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true)
        .selected
        .remove(0);
    let hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let older = expectation_record(
        &config.agent,
        &expectation,
        RESULT_PASS,
        "yes",
        hash.clone(),
    );
    append_history_record(&root, &expectation, &older).unwrap();
    let newer = expectation_record(
        &config.agent,
        &expectation,
        RESULT_FAIL,
        "Yes: concrete bug",
        hash,
    );
    append_history_record(&root, &expectation, &newer).unwrap();

    let record = reusable_history_record(&root, &config.agent, &expectation)
        .unwrap()
        .unwrap();

    assert_eq!(record.observed, "yes");
    assert!(record.passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_allows_missing_cache_key() {
    let root = git_project("history-cache-key-metadata");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.cache_key = None;
    append_history_record(&root, &expectation, &record).unwrap();

    assert!(reusable_history_record(&root, &config.agent, &expectation)
        .unwrap()
        .unwrap()
        .passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn free_form_observed_history_answers_are_reusable() {
    let mut record = sample_record(1, RESULT_FAIL);
    record.expected = Some("maybe".to_string());
    record.observed = "maybe".to_string();

    assert!(!record_requires_human_review(&record));
    assert!(is_reusable_history_record(&record));

    record.observed = "Rust".to_string();
    assert!(!record_requires_human_review(&record));
    assert!(is_reusable_history_record(&record));

    record.observed = "z".to_string();
    assert!(!record_requires_human_review(&record));
    assert!(is_reusable_history_record(&record));

    record.observed = "malformed".to_string();
    assert!(record_requires_human_review(&record));
    assert!(!is_reusable_history_record(&record));

    record.observed = "yes\nno".to_string();
    assert!(record_requires_human_review(&record));
    assert!(!is_reusable_history_record(&record));

    record.observed = format!("yes{}no", '\u{2028}');
    assert!(record_requires_human_review(&record));
    assert!(!is_reusable_history_record(&record));
}

#[test]
fn yes_no_history_answers_reject_free_form_answer_shape() {
    let mut record = sample_record(1, RESULT_FAIL);
    record.expected = Some("no".to_string());
    record.observed = "Yes: concrete bug".to_string();

    assert!(record_requires_human_review(&record));
    assert!(!is_reusable_history_record(&record));

    record.observed = "yes".to_string();
    assert!(!record_requires_human_review(&record));
    assert!(is_reusable_history_record(&record));

    record.expected = Some("a".to_string());
    record.observed = "b".to_string();
    assert!(!record_requires_human_review(&record));
    assert!(is_reusable_history_record(&record));

    record.observed = "Rust".to_string();
    assert!(record_requires_human_review(&record));
    assert!(!is_reusable_history_record(&record));
}

#[test]
fn reusable_history_record_uses_scope_hash_not_cache_key_for_changed_evaluator_config() {
    let root = git_project("history-cache-key-not-reuse-gate");
    let old_config = parse_check_config(check_config_yaml()).unwrap();
    let new_config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  thinking: medium
  instructions: |
    Different evaluator instructions.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let old_expectation = check_options(&old_config, &["1"], false, true).selected[0].clone();
    let new_expectation = check_options(&new_config, &["1"], false, true).selected[0].clone();
    append_history_record(
        &root,
        &old_expectation,
        &expectation_record(
            &old_config.agent,
            &old_expectation,
            "pass",
            "yes",
            staged_scope_hash(&root, &new_config.agent, &full_scope()).unwrap(),
        ),
    )
    .unwrap();

    assert!(
        reusable_history_record(&root, &new_config.agent, &new_expectation)
            .unwrap()
            .unwrap()
            .passed()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_respects_current_agent_ignore_patterns() {
    let root = git_project("history-reuse-ignore-dependent");
    let base_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let ignored_readme_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "README.md"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let old_expectation = check_options(&base_config, &["1"], false, true).selected[0].clone();
    let new_expectation =
        check_options(&ignored_readme_config, &["1"], false, true).selected[0].clone();
    append_history_record(
        &root,
        &old_expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: old_expectation.id.clone(),
            display_id: old_expectation.display_id.clone(),
            number: old_expectation.number,
            result: CheckResult::Pass,
            prompt: Some(old_expectation.q.clone()),
            expected: Some(old_expectation.a.clone()),
            observed: "yes".to_string(),
            evidence: "README.md answers it".to_string(),
            scope: vec!["README.md".to_string()],
            scope_hash: staged_scope_hash(&root, &base_config.agent, &["README.md".to_string()])
                .unwrap(),
            cache_key: Some(history_cache_key(&base_config.agent, &old_expectation)),
        },
    )
    .unwrap();

    let record =
        reusable_history_record(&root, &ignored_readme_config.agent, &new_expectation).unwrap();

    assert!(record.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_cache_key_ignores_agent_ignore_order() {
    let first_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "target/**"
    - "logs/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let second_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "logs/**"
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let first_expectation = check_options(&first_config, &["1"], false, true).selected[0].clone();
    let second_expectation = check_options(&second_config, &["1"], false, true).selected[0].clone();

    assert_eq!(
        history_cache_key(&first_config.agent, &first_expectation),
        history_cache_key(&second_config.agent, &second_expectation)
    );
}

#[test]
fn scope_hash_depends_on_agent_ignore_patterns() {
    let root = git_project("history-scope-hash-ignore-dependent");
    let base_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let ignored_readme_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "README.md"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();

    assert_ne!(
        staged_scope_hash(&root, &base_config.agent, &full_scope()).unwrap(),
        staged_scope_hash(&root, &ignored_readme_config.agent, &full_scope()).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_excludes_evaluator_denied_canon_paths() {
    let root = git_project("history-scope-hash-excludes-canon");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let before = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();

    let after = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();

    assert_eq!(before, after);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_excludes_nested_ignored_descendants_from_parent_tree_oid() {
    let root = git_project("history-scope-hash-nested-ignore");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "src/generated/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn visible() {}\n").unwrap();
    fs::write(
        root.join("src/generated/schema.rs"),
        "pub fn generated() {}\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "src"])
        .current_dir(&root)
        .output()
        .unwrap();

    let before = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    fs::write(
        root.join("src/generated/schema.rs"),
        "pub fn generated_changed() {}\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "src/generated/schema.rs"])
        .current_dir(&root)
        .output()
        .unwrap();
    let ignored_change = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn visible_changed() {}\n").unwrap();
    Command::new("git")
        .args(["add", "src/lib.rs"])
        .current_dir(&root)
        .output()
        .unwrap();
    let visible_change = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();

    assert_eq!(before, ignored_change);
    assert_ne!(before, visible_change);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_matches_git_write_tree_for_full_scope() {
    let root = git_project("history-scope-tree-oid-write-tree");
    let config = parse_check_config(check_config_yaml()).unwrap();

    let output = Command::new("git")
        .args(["write-tree"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let git_tree = command_output_trimmed(&output.stdout, "git write-tree stdout").unwrap();

    assert_eq!(
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
        git_tree
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_uses_sha256_for_sha256_git_repositories() {
    let root = temp_home("history-scope-tree-oid-sha256");
    let init = Command::new("git")
        .args(["init", "--object-format=sha256"])
        .current_dir(&root)
        .output()
        .unwrap();
    if !init.status.success() {
        let _ = fs::remove_dir_all(root);
        return;
    }
    for args in [
        ["config", "core.autocrlf", "false"],
        ["config", "core.eol", "lf"],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git config failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(root.join("README.md"), "hello").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let add = Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let config = parse_check_config(check_config_yaml()).unwrap();

    let output = Command::new("git")
        .args(["write-tree"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let git_tree = command_output_trimmed(&output.stdout, "git write-tree stdout").unwrap();

    assert_eq!(git_tree.len(), 64);
    assert_eq!(
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
        git_tree
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn scope_hash_handles_newline_paths_without_line_splitting() {
    let root = git_project("history-scope-hash-newline-path");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let path = "line\nbreak.txt";
    fs::write(root.join(path), "first").unwrap();
    Command::new("git")
        .arg("add")
        .arg(path)
        .current_dir(&root)
        .output()
        .unwrap();

    let entries = staged_scope_entries(&root, &[path.to_string()]).unwrap();

    assert_eq!(entries.len(), 1);
    assert!(entries[0].ends_with(path));
    let before = staged_scope_hash(&root, &config.agent, &[path.to_string()]).unwrap();
    fs::write(root.join(path), "second").unwrap();
    Command::new("git")
        .arg("add")
        .arg(path)
        .current_dir(&root)
        .output()
        .unwrap();
    let after = staged_scope_hash(&root, &config.agent, &[path.to_string()]).unwrap();
    assert_ne!(before, after);
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn scope_hash_treats_git_pathspec_magic_as_literal_path() {
    let root = git_project("history-scope-hash-literal-pathspec");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let path = ":(literal)name.txt";
    fs::write(root.join(path), "first").unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("--literal-pathspecs")
        .args(["add", "--"])
        .arg(path)
        .output()
        .unwrap();

    let entries = staged_scope_entries(&root, &[path.to_string()]).unwrap();

    assert_eq!(entries.len(), 1);
    assert!(entries[0].ends_with(path));
    let before = staged_scope_hash(&root, &config.agent, &[path.to_string()]).unwrap();
    fs::write(root.join(path), "second").unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("--literal-pathspecs")
        .args(["add", "--"])
        .arg(path)
        .output()
        .unwrap();
    let after = staged_scope_hash(&root, &config.agent, &[path.to_string()]).unwrap();
    assert_ne!(before, after);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_entry_encodes_non_utf8_path_bytes() {
    let entry = normalize_index_metadata(
        "100644 0123456789012345678901234567890123456789 0",
        b"dir/nonutf8-\xff.txt",
    )
    .unwrap();

    assert!(entry.contains("\0raw-path-hex:"));
    assert_eq!(
        sha1_scope_tree_oid_from_entries(&[entry]).unwrap().len(),
        40
    );
}

#[test]
fn scope_hash_reuses_fully_covered_directory_oid() {
    let directory_oid = "1111111111111111111111111111111111111111";
    let child_oid = "2222222222222222222222222222222222222222";
    let dir_entry = format!("40000 {}\tdir", directory_oid);
    let child_entry = format!("100644 {}\tdir/file.txt", child_oid);
    let dir_only = sha1_scope_tree_oid_from_entries(std::slice::from_ref(&dir_entry)).unwrap();

    assert_eq!(
        dir_only,
        sha1_scope_tree_oid_from_entries(&[dir_entry.clone(), child_entry.clone()]).unwrap()
    );
    assert_eq!(
        dir_only,
        sha1_scope_tree_oid_from_entries(&[child_entry, dir_entry]).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn staged_scope_entries_reuse_git_tree_oid_for_fully_covered_directory() {
    let root = git_project("scope-hash-staged-tree-oid");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let root_tree = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("write-tree")
        .output()
        .unwrap();
    assert!(
        root_tree.status.success(),
        "{}",
        String::from_utf8_lossy(&root_tree.stderr)
    );
    let root_tree_oid = command_output_trimmed(&root_tree.stdout, "git write-tree stdout").unwrap();
    let src_tree = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("rev-parse")
        .arg(format!("{}:src", root_tree_oid))
        .output()
        .unwrap();
    assert!(
        src_tree.status.success(),
        "{}",
        String::from_utf8_lossy(&src_tree.stderr)
    );
    let src_tree_oid = command_output_trimmed(&src_tree.stdout, "git rev-parse stdout").unwrap();
    let scope = vec!["src".to_string()];
    let entries = staged_scope_entries(&root, &scope).unwrap();
    let src_tree_entry = format!("40000 {}\tsrc", src_tree_oid);

    assert!(entries.iter().any(|entry| entry == &src_tree_entry));
    assert_eq!(
        staged_scope_hash(&root, &config.agent, &scope).unwrap(),
        sha1_scope_tree_oid_from_entries(&[src_tree_entry]).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}
