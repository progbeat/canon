use super::*;

#[test]
fn index_lock_reports_active_lock_and_stale_age() {
    with_env("index-active-lock", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        let _lock = lock_index(&config).unwrap();

        let err = upsert_index(&config, "hash", "one").unwrap_err();

        assert!(err.contains("index.tsv.lock"));
        assert!(err.contains("lock is already held"));
        assert!(!stale_index_lock_age(Duration::from_secs(
            INDEX_LOCK_STALE_AFTER_SECS - 1
        )));
        assert!(stale_index_lock_age(Duration::from_secs(
            INDEX_LOCK_STALE_AFTER_SECS
        )));
    });
}

#[test]
fn restore_note_reports_rollback_failure() {
    with_env("note-restore-failure", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        let path = config.root.join("blocked");
        fs::create_dir(&path).unwrap();

        let err = restore_deleted_note_after_index_failure(&path, b"original").unwrap_err();

        assert!(err.contains("failed to restore"));
        assert!(err.contains("after index update failure"));
    });
}

#[test]
fn index_updates_do_not_drop_hash_collisions() {
    with_env("index-collision", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        fs::write(
            config.root.join("index.tsv"),
            "samehash\tother-key\noldhash\ttarget-key\n",
        )
        .unwrap();

        upsert_index(&config, "samehash", "target-key").unwrap();
        let index = read_index(&config.root.join("index.tsv")).unwrap();

        assert!(index
            .iter()
            .any(|(hash, key)| hash == "samehash" && key == "other-key"));
        assert!(!index
            .iter()
            .any(|(hash, key)| hash == "oldhash" && key == "target-key"));
        assert!(index
            .iter()
            .any(|(hash, key)| hash == "samehash" && key == "target-key"));

        remove_index(&config, "samehash", "target-key").unwrap();
        let index = read_index(&config.root.join("index.tsv")).unwrap();
        assert!(index
            .iter()
            .any(|(hash, key)| hash == "samehash" && key == "other-key"));
        assert!(!index
            .iter()
            .any(|(hash, key)| hash == "samehash" && key == "target-key"));
    });
}

#[test]
fn index_updates_are_append_only_and_materialized_by_reader() {
    with_env("index-append-only", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();

        upsert_index(&config, "hash-one", "target-key").unwrap();
        upsert_index(&config, "hash-two", "target-key").unwrap();
        remove_index(&config, "hash-two", "target-key").unwrap();

        let raw = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert_eq!(raw.lines().count(), 3);
        assert!(read_index(&config.root.join("index.tsv"))
            .unwrap()
            .is_empty());

        upsert_index(&config, "hash-three", "target-key").unwrap();
        let raw = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert_eq!(raw.lines().count(), 4);
        assert_eq!(
            read_index(&config.root.join("index.tsv")).unwrap(),
            vec![("hash-three".to_string(), "target-key".to_string())]
        );
    });
}

#[test]
fn index_updates_compact_after_threshold() {
    with_env("index-append-compact", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        let writes = (INDEX_COMPACT_MIN_BYTES / 16) as usize;
        for index in 0..writes {
            upsert_index(&config, &format!("hash-{:08}", index), "target-key").unwrap();
        }
        upsert_index(&config, "hash-final", "target-key").unwrap();

        let raw = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert!(raw.lines().count() < writes);
        assert_eq!(
            read_index(&config.root.join("index.tsv")).unwrap(),
            vec![("hash-final".to_string(), "target-key".to_string())]
        );
    });
}

#[test]
fn read_index_rejects_malformed_lines() {
    with_env("bad-index", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        let path = config.root.join("index.tsv");
        fs::write(&path, "missing-tab\n").unwrap();

        let err = read_index(&path).unwrap_err();

        assert!(err.contains("malformed index line 1"));
    });
}

#[test]
fn note_index_rejects_control_characters() {
    assert!(validate_note_key("bad\u{7}key").is_err());
    assert!(validate_index_entry("bad\u{7}hash", "key").is_err());
    assert!(validate_index_entry("hash", "bad\u{7}key").is_err());
}

#[test]
fn collision_metadata_mismatch_fails() {
    with_env("collision", |_| {
        let config = Config::from_env().unwrap();
        let note = note_for_key(&config, "expected").unwrap();
        ensure_dir(&config.root).unwrap();
        fs::write(&note.path, header("actual", &note.hash)).unwrap();
        let result = ensure_note(&config, "expected");
        assert!(result.is_err());
    });
}

#[test]
fn header_parser_rejects_unknown_escape_sequences() {
    assert_eq!(
        parse_key_from_header(r#"<!-- canon key="bad\xkey" hash="hash" -->"#),
        None
    );
}
