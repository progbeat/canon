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
    assert!(parse_scope_json(r#"["target/output.txt"]"#, &config.agent).is_err());
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
fn scope_paths_treat_wildcard_characters_as_literal_filename_bytes() {
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
fn evaluator_session_key_is_not_newline_ambiguous() {
    assert_ne!(
        evaluator_session_key(&["a\nb".to_string(), "c".to_string()]),
        evaluator_session_key(&["a".to_string(), "b\nc".to_string()])
    );
}

#[test]
fn evaluator_response_scope_rejects_denied_paths() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    assert!(parse_scope_strings(&[".canon/check.yml".to_string()], &config.agent).is_err());
    assert!(parse_scope_strings(
        &["src/main.rs".to_string(), "target/output.txt".to_string()],
        &config.agent,
    )
    .is_err());
    assert!(parse_scope_strings(
        &[".".to_string(), "target/output.txt".to_string()],
        &config.agent,
    )
    .is_err());
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
    assert!(parse_scope_strings(&["foo/bar/baz.rs".to_string()], &config.agent).is_err());
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
fn agent_ignore_patterns_match_single_segment_wildcards() {
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
    assert!(!is_denied_path(&config.agent, "logs/nested/app.log"));
    assert!(is_denied_path(&config.agent, "src/a*b.txt"));
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
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut state = InterrogationState::new();
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
    assert_eq!(result.record.scope, vec!["src"]);
    let _ = fs::remove_dir_all(root);
}
