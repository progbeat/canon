use super::*;

#[test]
fn scope_is_canonicalized() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = parse_scope_json(
        r#"["src/main.rs", "README.md", "src", "README.md"]"#,
        &config.agent,
    )
    .unwrap();
    assert_eq!(scope, vec!["README.md", "src"]);
    let many_paths = parse_scope_json(r#"["a", "b", "c", "d", "e"]"#, &config.agent).unwrap();
    assert_eq!(many_paths, vec!["a", "b", "c", "d", "e"]);
    assert!(parse_scope_json(r#"[]"#, &config.agent).is_err());
}

#[test]
fn repo_paths_preserve_leading_and_trailing_spaces() {
    let config = parse_check_config(check_config_yaml()).unwrap();

    assert_eq!(normalize_repo_path(" leading.txt").unwrap(), " leading.txt");
    assert_eq!(
        normalize_repo_path("trailing.txt ").unwrap(),
        "trailing.txt "
    );
    assert_eq!(
        parse_scope_strings(&[" spaced.txt ".to_string()], &config.agent).unwrap(),
        vec![" spaced.txt ".to_string()]
    );
}

#[test]
fn repo_paths_reject_nul_before_process_boundaries() {
    let config = parse_check_config(check_config_yaml()).unwrap();

    let scope_err = parse_scope_strings(&["src/\0main.rs".to_string()], &config.agent).unwrap_err();
    assert!(scope_err.contains("NUL"));

    let hash_err = sanitize_scope_for_hash(&["src/\0main.rs".to_string()]).unwrap_err();
    assert!(hash_err.contains("NUL"));
}

#[test]
fn scope_paths_preserve_git_pathspec_characters() {
    let config = parse_check_config(check_config_yaml()).unwrap();

    assert_eq!(
        parse_scope_strings(
            &["src/what?.rs".to_string(), "data/*literal.txt".to_string()],
            &config.agent,
        )
        .unwrap(),
        vec!["data/*literal.txt".to_string(), "src/what?.rs".to_string()]
    );
    assert_eq!(
        sanitize_scope_for_hash(&["src/what?.rs".to_string()]).unwrap(),
        vec!["src/what?.rs".to_string()]
    );
}

#[test]
fn scope_pathspec_wildcards_match_git_paths() {
    let default_wildcard = vec!["src/*.rs".to_string()];
    assert!(path_bytes_in_scope(b"src/main.rs", &default_wildcard));
    assert!(path_bytes_in_scope(b"src/bin/main.rs", &default_wildcard));
    assert!(!path_bytes_in_scope(b"src/main.txt", &default_wildcard));

    let glob_magic = vec![":(glob)src/*.rs".to_string()];
    assert!(path_bytes_in_scope(b"src/main.rs", &glob_magic));
    assert!(!path_bytes_in_scope(b"src/bin/main.rs", &glob_magic));
}

#[test]
fn strict_scope_subset_canonicalizes_before_comparing() {
    assert!(!is_strict_scope_subset(
        &[".".to_string(), "src".to_string()],
        &[".".to_string()]
    ));
    assert!(!is_strict_scope_subset(
        &["src".to_string(), "src/main.rs".to_string()],
        &["src".to_string()]
    ));
    assert!(is_strict_scope_subset(
        &["src/main.rs".to_string()],
        &["src".to_string()]
    ));
}

#[test]
fn scope_containment_normalizes_repo_paths_before_comparing() {
    assert!(scope_contains("./src", "src/main.rs"));
    assert!(scope_contains("src", "./src/main.rs"));
    assert!(scope_is_within(
        &["./src/main.rs".to_string()],
        &["src".to_string()]
    ));
    assert!(scope_is_within(
        &["src/main.rs".to_string()],
        &["./src".to_string()]
    ));
    assert!(!is_strict_scope_subset(
        &["./src".to_string()],
        &["src".to_string()]
    ));
    assert!(is_strict_scope_subset(
        &["./src/main.rs".to_string()],
        &["src".to_string()]
    ));
    assert!(!scope_is_within(
        &["../src/main.rs".to_string()],
        &["src".to_string()]
    ));
    assert!(!scope_is_within(
        &[".".to_string(), "../secret.txt".to_string()],
        &[".".to_string()]
    ));
    assert!(!is_strict_scope_subset(
        &["src/main.rs".to_string()],
        &[".".to_string(), "../secret.txt".to_string()]
    ));
}

#[test]
fn evaluator_thread_reuse_key_is_not_newline_ambiguous() {
    let agent = parse_check_config(check_config_yaml()).unwrap().agent;
    assert_ne!(
        evaluator_thread_reuse_key(&agent, &["a\nb".to_string(), "c".to_string()], None, "tree"),
        evaluator_thread_reuse_key(&agent, &["a".to_string(), "b\nc".to_string()], None, "tree")
    );
    assert_ne!(
        evaluator_thread_reuse_key(&agent, &[".".to_string()], None, "tree-a"),
        evaluator_thread_reuse_key(&agent, &[".".to_string()], None, "tree-b")
    );
}

#[test]
fn evaluator_response_scope_keeps_ignored_paths_as_valid_scope_entries() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    assert_eq!(
        parse_scope_strings(&[".canon/check.yml".to_string()], &config.agent).unwrap(),
        vec![".canon/check.yml".to_string()]
    );
    assert_eq!(
        parse_scope_strings(
            &["src/main.rs".to_string(), "target/output.txt".to_string()],
            &config.agent,
        )
        .unwrap(),
        vec!["src/main.rs".to_string(), "target/output.txt".to_string()]
    );
    assert_eq!(
        parse_scope_strings(
            &[".".to_string(), "target/output.txt".to_string()],
            &config.agent,
        )
        .unwrap(),
        full_scope()
    );
}

#[test]
fn ignored_scope_paths_are_excluded_from_materialized_visible_tree() {
    let root = git_project("ignored-scope-materialized-visible-tree");
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(root.join("target/output.txt"), "ignored\n").unwrap();
    fs::write(root.join("src/main.rs"), "main\n").unwrap();
    Command::new("git")
        .args(["add", "target/output.txt", "src/main.rs"])
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = parse_scope_strings(
        &["src/main.rs".to_string(), "target/output.txt".to_string()],
        &config.agent,
    )
    .unwrap();

    let staged_view = StagedWorktreeView::apply(&root).unwrap();
    let scope_root = staged_view
        .materialize_evaluator_scope(&config.agent, &scope)
        .unwrap();

    assert!(scope_root.join("src/main.rs").exists());
    assert!(!scope_root.join("target/output.txt").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn full_scope_is_logical_and_still_keeps_denied_paths_denied() {
    let config = parse_check_config(check_config_yaml()).unwrap();

    assert_eq!(
        parse_scope_strings(&[".".to_string()], &config.agent).unwrap(),
        full_scope()
    );
    assert!(is_denied_path(&config.agent, ".canon/check.yml"));
    assert!(is_denied_path(&config.agent, ".git/canon/logs/0.jsonl"));
    assert!(is_denied_path(&config.agent, "target/output.txt"));
}

#[test]
fn agent_ignore_patterns_are_normalized_before_runtime_matching() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "foo/./bar/**"
  plugins: []
expectations:
  - q: "Does this behavior work?"
    a: "yes"
"#,
    )
    .unwrap();

    assert_eq!(config.agent.ignore, vec!["foo/bar/**"]);
    assert_eq!(
        parse_scope_strings(&["foo/bar/baz.rs".to_string()], &config.agent).unwrap(),
        vec!["foo/bar/baz.rs".to_string()]
    );
}

#[test]
fn runtime_ignore_pattern_normalization_fails_closed_for_invalid_patterns() {
    let agent = AgentConfig {
        models: Vec::new(),
        thinking: "low".to_string(),
        instructions: Some("x".to_string()),
        ignore: vec!["../secrets/**".to_string()],
        plugins: Vec::new(),
    };

    assert!(is_denied_path(&agent, "README.md"));
    assert!(is_denied_path(&agent, "src/main.rs"));
}

#[test]
fn agent_ignore_patterns_preserve_leading_and_trailing_spaces() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - " leading.txt"
    - "trailing.txt "
  plugins: []
expectations:
  - q: "Does this behavior work?"
    a: "yes"
"#,
    )
    .unwrap();

    assert_eq!(
        config.agent.ignore,
        vec![" leading.txt".to_string(), "trailing.txt ".to_string()]
    );
    assert!(is_denied_path(&config.agent, " leading.txt"));
    assert!(is_denied_path(&config.agent, "trailing.txt "));
    assert!(!is_denied_path(&config.agent, "leading.txt"));
    assert!(!is_denied_path(&config.agent, "trailing.txt"));
}

#[test]
fn agent_ignore_patterns_use_git_pathspec_wildcards() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "logs/*"
    - "src/a*b.txt"
  plugins: []
expectations:
  - q: "Does this behavior work?"
    a: "yes"
"#,
    )
    .unwrap();

    assert!(is_denied_path(&config.agent, "logs/app.log"));
    assert!(is_denied_path(&config.agent, "logs/nested/app.log"));
    assert!(is_denied_path(&config.agent, "src/a*b.txt"));
}

#[test]
fn agent_ignore_string_matching_normalizes_paths_and_matches_unicode_scalars() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "docs/?.md"
    - "foo/bar/**"
  plugins: []
expectations:
  - q: "Does this behavior work?"
    a: "yes"
"#,
    )
    .unwrap();

    assert!(is_denied_path(&config.agent, "./docs/é.md"));
    assert!(is_denied_path(&config.agent, "foo/./bar/baz.rs"));
    assert!(!is_denied_path(&config.agent, "docs/ab.md"));
    assert!(!is_denied_path(&config.agent, "foo/barbaz.rs"));
}

#[test]
fn denied_path_matching_handles_non_utf8_bytes() {
    let config = parse_check_config(check_config_yaml()).unwrap();

    assert!(is_denied_path_bytes(
        &config.agent,
        b"target/nonutf8-\xff.o"
    ));
    assert!(is_denied_path_bytes(
        &config.agent,
        b"./.canon/nonutf8-\xff.yml"
    ));
    assert!(!is_denied_path_bytes(&config.agent, b"src/nonutf8-\xff.rs"));
}

#[test]
fn project_wide_quality_scope_policy_is_not_runtime_rewritten() {
    let root = git_project("quality-scope-not-rewritten");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Are there any dirty hacks that can be avoided?"
    a: "no"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("no", "src looked clean", &["src"])]);
    let runtime = CheckRuntime::fixed(&root, &root, &config);
    let mut state = InterrogationRunState::new();
    let result = interrogate_expectation_with_model_fallbacks(
        &runtime,
        &options.selected[0],
        &mut runner,
        &mut None,
        &mut state,
        &full_scope(),
    )
    .unwrap();

    assert!(result.record.passed());
    assert_eq!(result.record.scope, full_scope());
    let _ = fs::remove_dir_all(root);
}
