use super::*;

#[test]
fn log_timestamp_uses_utc_rfc3339_format() {
    assert_eq!(format_record_timestamp(0), "1970-01-01T00:00:00Z");
}

#[test]
fn diagnostic_log_is_written_to_numeric_active_file_and_flushed() {
    let root = git_project("check-log");
    enable_diagnostic_logs(&root);
    let records = vec![sample_record(1, "pass")];
    let path = write_diagnostic_log(&root, &records).unwrap();
    assert_eq!(path, root.join(".git/canon/logs/0.jsonl"));
    let content = fs::read_to_string(&path).unwrap();
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let json: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(json["result"], "pass");
    assert_eq!(json["id"], expectation_id("Question?", "yes"));
    assert!(json.get("display_id").is_none());
    assert!(json.get("displayId").is_none());
    assert!(json.get("number").is_none());
    assert_eq!(json["prompt"], "Question?");
    assert_eq!(json["expected"], "yes");
    assert_eq!(json["observed"], "yes");
    assert_eq!(json["evidence"], "README.md has evidence");
    assert_eq!(json["scope"], json!(["."]));
    assert!(json.get("scopeTreeOid").is_none());
    let expected_order = [
        "\"timestamp\"",
        "\"id\"",
        "\"result\"",
        "\"observed\"",
        "\"evidence\"",
        "\"scope\"",
        "\"prompt\"",
        "\"expected\"",
    ];
    let mut previous = 0;
    for key in expected_order {
        let index = lines[0].find(key).unwrap();
        assert!(index >= previous);
        previous = index;
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_log_result_keeps_raw_review_diagnostics() {
    let root = git_project("runtime-log-human-review");
    enable_diagnostic_logs(&root);
    let mut wrong_record = sample_record(1, "fail");
    wrong_record.observed = "no".to_string();
    let mut review_record = sample_record(2, "fail");
    review_record.observed = UNPARSEABLE_OBSERVED.to_string();
    let path = write_diagnostic_log(&root, &[wrong_record, review_record]).unwrap();

    let events = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(events[0]["result"], "fail");
    assert_eq!(events[0]["expected"], "yes");
    assert_eq!(events[0]["observed"], "no");
    assert_eq!(events[1]["result"], "fail");
    assert_eq!(events[1]["expected"], "yes");
    assert_eq!(events[1]["observed"], UNPARSEABLE_OBSERVED);
    assert_eq!(events[1]["evidence"], "README.md has evidence");
    assert_eq!(events[2]["event"], "expectation.review_required");
    assert_eq!(events[2]["observed"], UNPARSEABLE_OBSERVED);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_rotates_at_start_when_active_file_is_large() {
    let root = git_project("check-log-rotate");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "1M"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config(&root).unwrap();
    assert_eq!(config.max_bytes, 1024 * 1024);
    assert_eq!(config.files.len(), 8);
    let active_limit = config.max_bytes / config.files.len() as u64;
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((active_limit + 1) as usize),
    )
    .unwrap();
    for (index, file_name) in config.files.iter().enumerate().skip(1) {
        fs::write(log_dir.join(file_name), format!("old-{index}")).unwrap();
    }

    let writer = DiagnosticLogWriter::create(&root).unwrap();
    assert_eq!(writer.path(), log_dir.join("0.jsonl").as_path());
    assert!(!log_dir.join("0.jsonl").exists());
    assert_eq!(
        fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
        (active_limit + 1) as usize
    );
    for (index, file_name) in config.files.iter().enumerate().skip(2) {
        assert_eq!(
            fs::read_to_string(log_dir.join(file_name)).unwrap(),
            format!("old-{}", index - 1)
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_runtime_log_event_rotates_before_writing() {
    let root = git_project("runtime-log-append-rotate");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "1M"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config(&root).unwrap();
    let active_limit = config.max_bytes / config.files.len() as u64;
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((active_limit + 1) as usize),
    )
    .unwrap();

    append_runtime_log_event(&root, "error", "worktree.restore.failed", &[]).unwrap();

    let active = fs::read_to_string(log_dir.join("0.jsonl")).unwrap();
    assert!(active.contains(r#""event":"worktree.restore.failed""#));
    assert_eq!(
        fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
        (active_limit + 1) as usize
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_writer_rotates_before_each_write() {
    let root = git_project("runtime-log-writer-rotate-each-write");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "1024"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");
    let mut writer = DiagnosticLogWriter::create(&root).unwrap();
    let payload = "x".repeat(180);

    writer
        .write_event("info", "first.write", &[("payload", json!(payload))])
        .unwrap();
    writer.write_event("info", "second.write", &[]).unwrap();

    let active = fs::read_to_string(log_dir.join("0.jsonl")).unwrap();
    let rotated = fs::read_to_string(log_dir.join("1.jsonl")).unwrap();
    assert!(active.contains(r#""event":"second.write""#));
    assert!(!active.contains(r#""event":"first.write""#));
    assert!(rotated.contains(r#""event":"first.write""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runtime_log_event_rejects_reserved_extra_fields() {
    let err = render_runtime_log_event("info", "event.test", &[("event", json!("override"))])
        .unwrap_err();

    assert!(err.to_string().contains("reserved"));
}

#[test]
fn runtime_log_event_rejects_duplicate_extra_fields() {
    let err = render_runtime_log_event(
        "info",
        "event.test",
        &[("id", json!("first")), ("id", json!("second"))],
    )
    .unwrap_err();

    assert!(err.to_string().contains("duplicated"));
}

#[test]
fn runtime_log_event_requires_known_schema_fields() {
    let err = render_runtime_log_event("info", "thread.start", &[("threadId", json!("thread-1"))])
        .unwrap_err();

    assert!(err.to_string().contains("scope"));
}

#[test]
fn runtime_log_event_validates_agent_response_token_usage_shape() {
    let err = render_runtime_log_event(
        "info",
        "agent.response",
        &[
            ("id", json!("id-1")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("response", json!({"text": "ok"})),
            ("tokenUsage", json!({"totalTokens": 1})),
        ],
    )
    .unwrap_err();

    assert!(err.to_string().contains("inputTokens"));
}

#[test]
fn runtime_log_event_rejects_null_agent_response_token_usage() {
    let err = render_runtime_log_event(
        "info",
        "agent.response",
        &[
            ("id", json!("id-1")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("response", json!({"text": "ok"})),
            ("tokenUsage", Value::Null),
        ],
    )
    .unwrap_err();

    assert!(err.to_string().contains("not an object"));
}

#[test]
fn diagnostic_log_lock_stale_age_has_explicit_threshold() {
    assert!(!stale_diagnostic_log_lock_age(Duration::from_secs(299)));
    assert!(stale_diagnostic_log_lock_age(Duration::from_secs(300)));
}

#[test]
fn diagnostic_log_lock_write_failure_removes_fresh_lock_file() {
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("write failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let root = git_project("runtime-log-lock-write-failure");
    let lock_path = root.join(".git/canon/logs/.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    fs::write(&lock_path, "").unwrap();

    let err = write_diagnostic_log_lock_token(&lock_path, &mut FailingWriter, "token").unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(!lock_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_config_reads_git_max_size() {
    let root = git_project("runtime-log-config");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "2M"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());

    let config = diagnostic_log_config(&root).unwrap();

    assert_eq!(config.max_bytes, 2 * 1024 * 1024);
    assert_eq!(config.files.len(), 8);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_config_uses_documented_disabled_default() {
    let root = git_project("runtime-log-config-default");

    let config = diagnostic_log_config(&root).unwrap();

    assert_eq!(config.max_bytes, 0);
    assert!(config.explicitly_disabled);
    assert_eq!(config.files.len(), 8);
    append_runtime_log_event(&root, "info", "default.disabled", &[]).unwrap();
    assert!(!root.join(".git/canon/logs/0.jsonl").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_config_disables_writes_for_explicit_zero_max_size() {
    let root = git_project("runtime-log-config-zero");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "0"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());

    let config = diagnostic_log_config(&root).unwrap();

    assert_eq!(config.max_bytes, 0);
    assert!(config.explicitly_disabled);
    append_runtime_log_event(
        &root,
        "info",
        "disabled.write",
        &[("payload", json!("x".repeat(2048)))],
    )
    .unwrap();
    assert!(!root.join(".git/canon/logs/0.jsonl").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_rotation_prunes_to_configured_total_size() {
    let root = git_project("runtime-log-total-size");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "1024"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config(&root).unwrap();
    let active_limit = config.max_bytes / config.files.len() as u64;
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((active_limit + 1) as usize),
    )
    .unwrap();
    for file_name in config.files.iter().skip(1) {
        fs::write(log_dir.join(file_name), "x".repeat(300)).unwrap();
    }

    let writer = DiagnosticLogWriter::create(&root).unwrap();

    assert_eq!(writer.path(), log_dir.join("0.jsonl").as_path());
    let total: u64 = config
        .files
        .iter()
        .map(|file_name| log_dir.join(file_name))
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.len())
        .sum();
    assert!(total <= config.max_bytes);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_runtime_log_event_prunes_after_writing() {
    let root = git_project("runtime-log-post-write-prune");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "512"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config(&root).unwrap();
    fs::write(log_dir.join("1.jsonl"), "x".repeat(450)).unwrap();

    append_runtime_log_event(&root, "info", "post.write.prune", &[]).unwrap();

    let total: u64 = config
        .files
        .iter()
        .map(|file_name| log_dir.join(file_name))
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.len())
        .sum();
    assert!(total <= config.max_bytes);
    assert!(fs::read_to_string(log_dir.join("0.jsonl"))
        .unwrap()
        .contains(r#""event":"post.write.prune""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_runtime_log_event_rejects_oversized_record() {
    let root = git_project("runtime-log-reject-oversized");
    let output = Command::new("git")
        .args(["config", "canon.logs.maxSize", "64"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let log_dir = root.join(".git/canon/logs");

    let err = append_runtime_log_event(
        &root,
        "info",
        "oversized.active",
        &[("payload", json!("x".repeat(160)))],
    )
    .unwrap_err();

    assert!(err.to_string().contains("too large"));
    assert!(!log_dir.join("0.jsonl").exists());
    let _ = fs::remove_dir_all(root);
}
