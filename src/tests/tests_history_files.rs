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
            .join(GIT_CANON_CACHE_DIR)
            .join(&expectation.id)
            .join(history_file_name())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_record_required_fields_are_written_first() {
    let record = sample_record(1, "pass");
    let line = render_check_log_record(&record).unwrap();
    let json: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(json["id"], expectation_id("Question?", "yes"));
    assert!(json.get("display_id").is_none());
    assert!(json.get("displayId").is_none());
    assert_eq!(json["prompt"], "Question?");
    assert_eq!(json["expected"], "yes");

    let expected_order = [
        "\"timestamp\"",
        "\"result\"",
        "\"observed\"",
        "\"evidence\"",
        "\"scope\"",
        "\"scopeTreeOid\"",
        "\"id\"",
        "\"prompt\"",
        "\"expected\"",
    ];
    let mut previous = 0;
    for key in expected_order {
        let index = line.find(key).unwrap();
        assert!(index >= previous);
        previous = index;
    }
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
            render_check_log_record(&sample_record(1, "pass"))
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
fn history_parser_accepts_legacy_required_prefix_records() {
    let line = serde_json::to_string(&json!({
        "timestamp": "1970-01-01T00:00:00Z",
        "result": "pass",
        "observed": "yes",
        "evidence": "cached answer",
        "scope": ["."],
        "scopeTreeOid": "AAAAAAAAAAAAAAAAAAAA"
    }))
    .unwrap();

    let record = parse_history_record_line(Path::new("history.jsonl"), 1, &line).unwrap();

    assert_eq!(record.prompt, None);
    assert_eq!(record.expected, None);
    assert_eq!(record.observed, "yes");
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
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record_with_cache(&root, &expectation, &record, &mut history_cache).unwrap();

    let cached = history_cache.read_records(&root, &expectation).unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].observed, "yes");
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
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
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
fn history_compaction_uses_one_in_fifteen_chance() {
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
    let records = (1..=7)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            render_check_log_record(&record)
                .unwrap()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>();
    fs::write(&path, format!("{}\n", records.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 3", "record 4", "record 5", "record 6", "record 7"]
    );
    assert!(!compact_history_temp_path(&path).unwrap().exists());
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
        render_check_log_record(&record)
            .unwrap()
            .trim_end()
            .to_string()
    }));
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 3", "record 4", "record 5", "record 6", "record 7"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_non_record_json_objects() {
    let root = git_project("history-compact-non-record");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{{\"n\":1}}\n{}\n",
            render_check_log_record(&sample_record(1, "pass"))
                .unwrap()
                .trim_end()
        ),
    )
    .unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 1);
    assert_eq!(compacted[0].id, expectation_id("Question?", "yes"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn git_path_from_raw_bytes_preserves_non_utf8_unix_paths() {
    use std::os::unix::ffi::OsStrExt;

    let path = git_path_from_raw_bytes(b"not-utf8-\xff.md").unwrap();
    assert_eq!(path.as_os_str().as_bytes(), b"not-utf8-\xff.md");
}
