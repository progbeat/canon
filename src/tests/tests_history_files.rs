use super::*;

#[test]
fn history_path_uses_expectation_id_directory() {
    let root = git_project("history-path");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1"], false, true);
    let expectation = options.selected.remove(0);
    assert_eq!(
        history_path(&root, &expectation).unwrap(),
        root.join(".git")
            .join(CANON_CACHE_DIR_GIT_PATH)
            .join(&expectation.id)
            .join(history_file_name())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_record_required_fields_are_written_first() {
    let record = sample_record(1, "pass");
    let line = render_answer_history_record(&record).unwrap();
    let json: Value = serde_json::from_str(&line).unwrap();
    assert!(json.get("id").is_none());
    assert!(json.get("display_id").is_none());
    assert!(json.get("displayId").is_none());
    assert!(json.get("prompt").is_none());
    assert!(json.get("expected").is_none());
    assert!(json.get("result").is_none());
    assert!(json.get("cacheKey").is_none());
    assert_eq!(json["qScope"], json!(["."]));
    assert!(json.get("scope").is_none());
    assert!(json.get("visibleTreeOid").is_some());
    assert!(json.get("scopeTreeOid").is_none());
    assert!(json.get("scopeHash").is_none());

    let expected_order = [
        "\"timestamp\"",
        "\"observed\"",
        "\"evidence\"",
        "\"qScope\"",
        "\"visibleTreeOid\"",
    ];
    let mut previous = 0;
    for key in expected_order {
        let index = line.find(key).unwrap();
        assert!(index >= previous);
        previous = index;
    }
}

#[test]
fn check_runner_history_record_uses_native_visible_tree_oid() {
    let root = git_project("history-visible-tree-oid-from-runner");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    let mut runner = FakeRunner::new(&[&answer("yes", "Full scope supports yes.", &["."])]);

    run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

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
    let records = read_history_records(&root, &expectation).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].visible_tree_oid, git_tree);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_cache_cleanup_removes_inactive_expectation_entries() {
    let root = git_project("history-cleanup-stale-cache");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let active_ids = active_expectation_ids(&config);
    let active_id = active_ids.iter().next().unwrap().clone();
    let cache_dir = root.join(".git/canon/cache");
    fs::create_dir_all(cache_dir.join(&active_id)).unwrap();
    fs::write(cache_dir.join(&active_id).join(history_file_name()), "").unwrap();
    fs::create_dir_all(cache_dir.join("stale-id")).unwrap();
    fs::write(cache_dir.join("stale-id").join(history_file_name()), "").unwrap();
    fs::write(cache_dir.join("stale-file"), "old").unwrap();

    let stats = cleanup_stale_cache_dirs(&cache_dir, &active_ids).unwrap();

    assert_eq!(stats.removed, 2);
    assert_eq!(stats.kept, 1);
    assert!(cache_dir.join(&active_id).exists());
    assert!(!cache_dir.join("stale-id").exists());
    assert!(!cache_dir.join("stale-file").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_reader_skips_malformed_lines() {
    let root = git_project("history-skips-malformed-json");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{{not json}}\n{}\n",
            render_answer_history_record(&sample_record(1, "pass"))
                .unwrap()
                .trim_end()
        ),
    )
    .unwrap();

    let records = read_history_records(&root, &expectation).unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].observed, "yes");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_parser_accepts_required_prefix_records() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "observed": "yes",
        "evidence": "cached answer",
        "qScope": ["."],
        "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }))
    .unwrap();

    let record = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap();

    assert_eq!(record.prompt, None);
    assert_eq!(record.expected, None);
    assert_eq!(record.result, CheckResult::Fail);
    assert_eq!(record.observed, "yes");
    assert_eq!(
        record.visible_tree_oid,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn history_parser_accepts_additional_history_fields() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "observed": "yes",
        "evidence": "cached answer",
        "qScope": ["."],
        "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "futureMetadata": {"source": "newer canon"}
    }))
    .unwrap();

    let record = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap();

    assert_eq!(record.observed, "yes");
    assert_eq!(record.scope, vec![".".to_string()]);
}

#[test]
fn history_parser_rejects_observed_values_outside_evaluator_answer_schema() {
    for observed in ["", "yes\nno", "yes\rno"] {
        let line = serde_json::to_string(&json!({
            "timestamp": "1970-01-01T00:00:00Z",
            "observed": observed,
            "evidence": "cached answer",
            "qScope": ["."],
            "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .unwrap();

        let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

        assert!(err.contains("evaluator response answer schema"), "{err}");
    }
}

#[test]
fn history_cache_reader_skips_non_native_visible_tree_oid_records() {
    let root = git_project("history-read-non-native-visible-tree-oid");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "observed": "yes",
        "evidence": "sha256-shaped oid in a sha1 repository",
        "qScope": ["."],
        "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }))
    .unwrap();
    fs::write(&path, format!("{line}\n")).unwrap();

    let mut history_cache = HistoryCache::new();
    let records = history_cache.read_records(&root, &expectation).unwrap();

    assert!(records.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_parser_rejects_error_records_as_answer_history() {
    let line = error_history_record_line();

    let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

    assert!(err.contains("error responses are not answer history records"));
}

#[test]
fn history_parser_rejects_null_error_field_as_answer_history() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "observed": "yes",
        "error": null,
        "evidence": "not an answer history row",
        "qScope": ["."],
        "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }))
    .unwrap();

    let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

    assert!(err.contains("error responses are not answer history records"));
}

#[test]
fn history_parser_rejects_legacy_scope_records() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "result": "pass",
        "observed": "yes",
        "evidence": "cached answer",
        "scope": ["."],
        "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }))
    .unwrap();

    let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

    assert!(err.contains("invalid history JSON"));
}

#[test]
fn history_parser_rejects_legacy_scope_tree_oid_records() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "result": "pass",
        "observed": "yes",
        "evidence": "cached answer",
        "scope": ["."],
        "scopeTreeOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    }))
    .unwrap();

    let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

    assert!(err.contains("invalid history JSON"));
}

#[test]
fn history_parser_rejects_legacy_scope_hash_records() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "result": "pass",
        "observed": "yes",
        "evidence": "cached answer",
        "scope": ["."],
        "scopeHash": "cccccccccccccccccccccccccccccccccccccccc"
    }))
    .unwrap();

    let err = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap_err();

    assert!(err.contains("invalid history JSON"));
}

#[test]
fn append_history_record_updates_in_memory_cache() {
    let root = git_project("history-cache-coherent");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1"], false, true);
    let expectation = options.selected.remove(0);
    let mut history_cache = HistoryCache::new();
    assert!(history_cache
        .read_records(&root, &expectation)
        .unwrap()
        .is_empty());

    let record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record_with_cache(&root, &expectation, &record, &mut history_cache).unwrap();

    let cached = history_cache.read_records(&root, &expectation).unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].observed, "yes");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_current_history_record_rejects_non_current_visible_tree_oid() {
    let root = git_project("history-append-current-visible-tree-oid");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let record = sample_record(1, "pass");
    let mut history_cache = HistoryCache::new();
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

    let err = append_current_history_record_with_cache(
        &root,
        &expectation,
        &record,
        &mut history_cache,
        &mut visible_tree_oid_cache,
    )
    .unwrap_err();

    assert!(err.contains("visibleTreeOid must match"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_history_record_rejects_non_answer_records() {
    let root = git_project("history-append-non-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.error = Some(ERROR_INSUFFICIENT_EVIDENCE.to_string());

    let err = append_history_record(&root, &expectation, &record).unwrap_err();

    assert!(err.contains("schema-valid responses with answer"), "{err}");
    assert!(!history_path(&root, &expectation).unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_history_record_rejects_non_schema_answer_values() {
    let root = git_project("history-append-invalid-answer-value");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.observed.clear();

    let err = append_history_record(&root, &expectation, &record).unwrap_err();

    assert!(err.contains("evaluator response answer schema"), "{err}");
    assert!(!history_path(&root, &expectation).unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn append_history_record_refuses_symlinked_history_file() {
    use std::os::unix::fs::symlink;

    let root = git_project("history-append-symlink");
    let outside = temp_home("history-append-symlink-target");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    let target = outside.join("target.txt");
    fs::write(&target, "outside\n").unwrap();
    symlink(&target, &path).unwrap();
    let record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );

    let err = append_history_record(&root, &expectation, &record).unwrap_err();

    assert!(err.contains("refusing to use symlink"), "{err}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[cfg(unix)]
#[test]
fn latest_non_pass_record_refuses_symlinked_state_file() {
    use std::os::unix::fs::symlink;

    let root = git_project("latest-non-pass-symlink");
    let outside = temp_home("latest-non-pass-symlink-target");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let history = history_path(&root, &expectation).unwrap();
    let path = history.parent().unwrap().join("latest-non-pass.json");
    ensure_dir(path.parent().unwrap()).unwrap();
    let target = outside.join("target.txt");
    fs::write(&target, "outside\n").unwrap();
    symlink(&target, &path).unwrap();
    let record = sample_record(1, "fail");

    let err = write_latest_non_pass_record(&root, &expectation, &record).unwrap_err();

    assert!(err.contains("refusing to use symlink"), "{err}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn history_compaction_uses_one_in_sixteen_chance() {
    assert!(should_compact_history_for_seed(0));
    assert!(!should_compact_history_for_seed(1));
    let hits = (0..(HISTORY_COMPACT_CHANCE_DENOMINATOR * 10))
        .filter(|seed| should_compact_history_for_seed(*seed))
        .count() as u64;
    assert_eq!(hits, 10);
    let _ = should_compact_history();
}

#[test]
fn compact_history_replaces_file_after_writing_latest_lines() {
    let root = git_project("history-compact");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let records = (1..=10)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            render_answer_history_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    fs::write(&path, format!("{}\n", records.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 8);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec![
            "record 3",
            "record 4",
            "record 5",
            "record 6",
            "record 7",
            "record 8",
            "record 9",
            "record 10",
        ]
    );
    assert!(!compact_history_temp_path(&path).unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_refuses_to_replace_while_history_lock_is_held() {
    let root = git_project("history-compact-lock-held");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let records = (1..=10)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            render_answer_history_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    fs::write(&path, format!("{}\n", records.join("\n"))).unwrap();
    let lock_path = path.with_file_name("history.jsonl.lock");
    fs::write(&lock_path, "held\n").unwrap();

    let err = compact_history(&path).unwrap_err();

    assert!(err.contains("lock is already held"), "{err}");
    assert_eq!(read_history_records_from_path(&path).unwrap().len(), 10);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_malformed_lines_and_keeps_latest_valid_records() {
    let root = git_project("history-compact-malformed");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut lines = vec!["not json".to_string()];
    lines.extend((1..=7).map(|number| {
        let mut record = sample_record(number, "pass");
        record.evidence = format!("record {number}");
        render_answer_history_record(&record)
            .unwrap()
            .trim_end()
            .to_string()
    }));
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 7);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 1", "record 2", "record 3", "record 4", "record 5", "record 6", "record 7",]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_non_history_json_objects() {
    let root = git_project("history-compact-non-history-json-object");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut lines = (1..=5)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            render_answer_history_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    lines.push("{\"n\":1}".to_string());
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let lines = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 1", "record 2", "record 3", "record 4", "record 5"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_repository_history_drops_non_native_visible_tree_oid_records() {
    let root = git_project("history-compact-non-native-visible-tree-oid");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut lines = (1..=8)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("native record {number}");
            render_answer_history_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    lines.push(
        serde_json::to_string(&json!({
            "timestamp": "1970-01-01T00:00:09Z",
            "observed": "yes",
            "evidence": "sha256-shaped oid in a sha1 repository",
            "qScope": ["."],
            "visibleTreeOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .unwrap(),
    );
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_repository_history(&root, &path).unwrap();

    let lines = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 8);
    assert!(lines.iter().all(|line| !line.contains("sha256-shaped oid")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_error_records() {
    let root = git_project("history-compact-error-record");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut lines = (1..=5)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            render_answer_history_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    lines.push(error_history_record_line());
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let lines = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert!(compacted.iter().all(|record| record.error.is_none()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_reader_skips_error_records() {
    let root = git_project("history-read-error-record");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut first = sample_record(1, "pass");
    first.evidence = "first".to_string();
    let mut second = sample_record(2, "pass");
    second.evidence = "second".to_string();
    let lines = [
        render_answer_history_record(&first)
            .unwrap()
            .trim_end()
            .to_string(),
        error_history_record_line(),
        render_answer_history_record(&second)
            .unwrap()
            .trim_end()
            .to_string(),
    ];
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let records = read_history_records_from_path(&path).unwrap();

    assert_eq!(
        records
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let _ = fs::remove_dir_all(root);
}

fn error_history_record_line() -> String {
    serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "result": "fail",
        "observed": ERROR_INVALID_QUESTION,
        "error": ERROR_INVALID_QUESTION,
        "evidence": "invalid question",
        "qScope": ["."],
        "visibleTreeOid": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    }))
    .unwrap()
}

#[cfg(unix)]
#[test]
fn git_path_from_raw_bytes_preserves_non_utf8_unix_paths() {
    use std::os::unix::ffi::OsStrExt;

    let path = git_path_from_raw_bytes(b"not-utf8-\xff.md").unwrap();
    assert_eq!(path.as_os_str().as_bytes(), b"not-utf8-\xff.md");
}
