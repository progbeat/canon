use super::*;

#[test]
fn cooldown_reuse_stops_at_latest_valid_failure() {
    let root = git_project("history-cooldown-latest-fail");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let visible_tree_oid = staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            error: None,
            evidence: "old pass".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: visible_tree_oid.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "no".to_string(),
            error: None,
            evidence: "latest fail".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: visible_tree_oid.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "not-a-timestamp".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "no".to_string(),
            error: None,
            evidence: "invalid timestamp fail".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: "newer".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    assert!(
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_skips_latest_legacy_non_answer_record() {
    let root = git_project("history-cooldown-latest-error");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let visible_tree_oid = staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            error: None,
            evidence: "old pass".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: visible_tree_oid.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_legacy_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: ERROR_INSUFFICIENT_EVIDENCE.to_string(),
            error: Some(ERROR_INSUFFICIENT_EVIDENCE.to_string()),
            evidence: "latest human review".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    );

    let mut history_cache = HistoryCache::new();
    let record =
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .unwrap();
    assert!(record.passed());
    assert_eq!(record.evidence, "old pass");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_accepts_future_dated_pass_as_fresh() {
    let root = git_project("history-cooldown-future-pass");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "2099-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            error: None,
            evidence: "future pass".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();

    assert!(
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .unwrap()
            .passed()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_recomputes_result_from_current_expected_answer() {
    let root = git_project("history-cooldown-current-result");
    let old_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "no"
    cooldown: 1d
"#,
    )
    .unwrap();
    let new_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let old_expectation = check_options(&old_config, &["1"], false, false).selected[0].clone();
    let new_expectation = check_options(&new_config, &["1"], false, false).selected[0].clone();
    let mut record = expectation_record(
        &old_config.agent,
        &old_expectation,
        "fail",
        "yes",
        staged_visible_tree_oid(&root, &old_config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "1970-01-01T00:00:10Z".to_string();
    append_history_record(&root, &old_expectation, &record).unwrap();

    let mut history_cache = HistoryCache::new();
    let reused = cooldown_history_record(
        &root,
        &new_config.agent,
        &new_expectation,
        &mut history_cache,
        30,
    )
    .unwrap()
    .unwrap();

    assert_eq!(reused.expected.as_deref(), Some("yes"));
    assert!(reused.passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_uses_fresh_pass_without_cache_key_filter() {
    let root = git_project("history-cooldown-no-cache-key-gate");
    let old_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let new_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 2d
"#,
    )
    .unwrap();
    let old_expectation = check_options(&old_config, &["1"], false, false).selected[0].clone();
    let new_expectation = check_options(&new_config, &["1"], false, false).selected[0].clone();
    let mut record = expectation_record(
        &old_config.agent,
        &old_expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &old_config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &old_expectation, &record).unwrap();

    let mut history_cache = HistoryCache::new();
    assert!(cooldown_history_record(
        &root,
        &new_config.agent,
        &new_expectation,
        &mut history_cache,
        unix_timestamp().unwrap(),
    )
    .unwrap()
    .unwrap()
    .passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_blocks_on_latest_answer_record_with_invalid_timestamp() {
    let root = git_project("history-cooldown-invalid-timestamp");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let visible_tree_oid = staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            error: None,
            evidence: "old pass".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "not-a-timestamp".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            error: None,
            evidence: "invalid timestamp pass".to_string(),
            scope: full_scope(),
            suggested_q_scope: None,
            visible_tree_oid: "new".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    assert!(
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_applies_current_expectation_metadata() {
    let root = git_project("history-cooldown-preserves-record");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let mut expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let visible_tree_oid = staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    let record = CheckRecord {
        timestamp: "1970-01-01T00:00:10Z".to_string(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
        number: 42,
        result: CheckResult::Pass,
        prompt: Some("Question?".to_string()),
        expected: Some("yes".to_string()),
        observed: "yes".to_string(),
        error: None,
        evidence: "old pass".to_string(),
        scope: full_scope(),
        suggested_q_scope: None,
        visible_tree_oid,
        cache_key: Some(history_cache_key(&config.agent, &expectation)),
    };
    append_history_record(&root, &expectation, &record).unwrap();
    expectation.number = 7;

    let mut history_cache = HistoryCache::new();
    let reused =
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .unwrap();

    assert_eq!(reused.id, expectation.id);
    assert_eq!(reused.number, 7);
    assert_eq!(reused.prompt.as_deref(), Some("Question?"));
    assert_eq!(reused.expected.as_deref(), Some("yes"));
    assert_eq!(reused.timestamp, "1970-01-01T00:00:10Z");
    let _ = fs::remove_dir_all(root);
}
