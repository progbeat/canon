use super::*;

#[test]
fn hash_is_ten_base64url_chars() {
    let hash = hash_key("src/lib.rs");
    assert_eq!(hash.len(), 10);
    assert!(hash
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
}

#[test]
fn path_creation_is_deterministic() {
    with_env("deterministic", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "a/b.rs").unwrap();
        let second = ensure_note(&config, "a/b.rs").unwrap();
        assert_eq!(first.path, second.path);
        assert!(first.path.exists());
    });
}

#[test]
fn write_and_append_preserve_metadata() {
    with_env("write-append", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(&config, "src/main.rs", "decision").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(content.starts_with("<!-- canon key=\"src/main.rs\" hash=\""));
        assert!(content.contains("\nbody\n"));
        assert!(content.contains("decision"));
    });
}

#[test]
fn write_replaces_visible_note_content_and_compacts_file() {
    with_env("write-replace", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "old body").unwrap();
        write_note(&config, "src/main.rs", "new body").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(!content.contains("old body"));
        assert!(content.contains("new body"));
        assert!(!raw.contains("old body"));
        assert!(!raw.contains("<!-- canon log v1 -->"));
    });
}

#[test]
fn append_persists_log_record_without_rewriting_note() {
    with_env("append-log", |_| {
        let config = Config::from_env().unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        ensure_dir(&config.root).unwrap();
        fs::write(
            &note.path,
            format!("{}body\n", initial_content(&note.key, &note.hash)),
        )
        .unwrap();

        append_note(&config, "src/main.rs", "decision").unwrap();

        let raw = fs::read_to_string(&note.path).unwrap();
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(content.contains("\nbody\n"));
        assert!(content.contains("decision"));
        assert!(raw.contains("<!-- canon log v1 -->"));
        assert!(raw.contains(r#""op":"append""#));
    });
}

#[test]
fn append_compacts_note_log_after_threshold() {
    with_env("append-log-compact", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(
            &config,
            "src/main.rs",
            &"decision".repeat((NOTE_LOG_COMPACT_MIN_BYTES / 8) as usize + 1),
        )
        .unwrap();

        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        assert!(!raw.contains("<!-- canon log v1 -->"));
        assert!(raw.contains("decision"));
    });
}

#[test]
fn materialize_note_content_ignores_marker_like_body_text() {
    let note = Note {
        key: "src/main.rs".to_string(),
        hash: hash_key("src/main.rs"),
        path: PathBuf::from("note.md"),
    };
    let raw = format!(
        "{}body\n<!-- canon log v1 -->\nordinary text\n",
        initial_content(&note.key, &note.hash)
    );

    let content = materialize_note_content(&note, &raw).unwrap();

    assert_eq!(content, raw);
}

#[test]
fn delete_removes_only_target() {
    with_env("delete", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "one").unwrap();
        let second = ensure_note(&config, "two").unwrap();
        delete_note(&config, "one").unwrap();
        assert!(!first.path.exists());
        assert!(second.path.exists());
        let index = read_index(&config.root.join("index.tsv")).unwrap();
        assert!(!index.iter().any(|(_, key)| key == "one"));
        assert!(index.iter().any(|(_, key)| key == "two"));
    });
}

#[test]
fn delete_verifies_note_before_removing_index_entry() {
    with_env("delete-bad-note", |_| {
        let config = Config::from_env().unwrap();
        let note = ensure_note(&config, "one").unwrap();
        fs::write(&note.path, "<!-- canon key=\"other\" hash=\"bad\" -->\n").unwrap();

        let err = delete_note(&config, "one").unwrap_err();

        assert!(err.contains("belongs to key"));
        assert!(note.path.exists());
        let index = read_index(&config.root.join("index.tsv")).unwrap();
        assert!(index.iter().any(|(_, key)| key == "one"));
    });
}

#[test]
fn collect_text_rejects_invalid_start_index() {
    let args = vec![OsString::from("one")];
    let err = collect_text(&args, 2).unwrap_err();
    assert!(err.contains("exceeds argument count"));
}

#[test]
fn note_keys_reject_index_separators() {
    with_env("bad-note-key", |_| {
        let config = Config::from_env().unwrap();
        assert!(write_note(&config, "bad\tkey", "body").is_err());
        assert!(write_note(&config, "bad\nkey", "body").is_err());
    });
}
