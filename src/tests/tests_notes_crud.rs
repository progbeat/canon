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
fn concurrent_appends_keep_note_log_records_materializable() {
    with_env("append-log-concurrent", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        let root = config.root.clone();

        let handles = (0..16)
            .map(|index| {
                let root = root.clone();
                std::thread::spawn(move || {
                    let config = Config { root };
                    append_note(&config, "src/main.rs", &format!("decision-{index:02}")).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(content.contains("\nbody\n"));
        for index in 0..16 {
            assert!(content.contains(&format!("decision-{index:02}")));
        }
    });
}

#[test]
fn failed_append_rollback_removes_partial_note_log_record() {
    with_env("append-log-rollback", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(&config, "src/main.rs", "kept").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let previous_size = fs::metadata(&note.path).unwrap().len();
        let partial = b"\n<!-- canon log v1 -->\n{\"op\":\"append\"";
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&note.path)
            .unwrap();
        file.write_all(partial).unwrap();
        file.flush().unwrap();

        rollback_note_log_append_for_test(&note.path, previous_size).unwrap();

        let raw = fs::read_to_string(&note.path).unwrap();
        assert_eq!(raw.len() as u64, previous_size);
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(content.contains("kept"));
        assert!(!content.contains(r#"{"op":"append""#));
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
fn append_succeeds_when_followup_compaction_rewrite_fails() {
    with_env("append-log-compact-fails", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let file_name = note.path.file_name().unwrap().to_str().unwrap();
        let temp_path = note
            .path
            .with_file_name(format!(".{}.{}.tmp", file_name, process::id()));
        fs::write(&temp_path, "block compaction temp create").unwrap();

        append_note(
            &config,
            "src/main.rs",
            &"decision".repeat((NOTE_LOG_COMPACT_MIN_BYTES / 8) as usize + 1),
        )
        .unwrap();

        let raw = fs::read_to_string(&note.path).unwrap();
        let content = materialize_note_content(&note, &raw).unwrap();
        assert!(raw.contains("<!-- canon log v1 -->"));
        assert!(content.contains("decision"));
        let _ = fs::remove_file(temp_path);
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
fn write_escapes_note_log_marker_collision() {
    with_env("write-marker-collision", |_| {
        let config = Config::from_env().unwrap();
        let body = "body\n<!-- canon log v1 -->\n{\"op\":\"write\",\"text\":\"x\"}";

        write_note(&config, "src/main.rs", body).unwrap();

        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        assert!(raw.contains("\n\\<!-- canon log v1 -->\n"));
        assert!(!raw.contains("\n<!-- canon log v1 -->\n{\"op\":\"write\""));
        let mut rendered = String::new();
        stream_note_content(&note, io::Cursor::new(raw.as_bytes()), |chunk| {
            rendered.push_str(chunk);
            Ok(())
        })
        .unwrap();
        assert!(rendered.contains(body));
    });
}

#[test]
fn read_streams_later_append_log_marker_lines_unescaped() {
    with_env("append-marker-stream", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(&config, "src/main.rs", "ok").unwrap();
        append_note(&config, "src/main.rs", "<!-- canon log v1 -->").unwrap();

        let note = note_for_key(&config, "src/main.rs").unwrap();
        let raw = fs::read_to_string(&note.path).unwrap();
        assert!(raw.contains("<!-- canon log v1 -->"));
        let mut rendered = String::new();
        stream_note_content(&note, io::Cursor::new(raw.as_bytes()), |chunk| {
            rendered.push_str(chunk);
            Ok(())
        })
        .unwrap();
        assert!(rendered.contains("\n<!-- canon log v1 -->\n"));
        assert!(!rendered.contains("\\<!-- canon log v1 -->"));
    });
}

#[cfg(unix)]
#[test]
fn append_note_refuses_symlinked_note_file() {
    use std::os::unix::fs::symlink;

    with_env("append-symlink", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let outside = temp_home("append-symlink-target");
        ensure_dir(&outside).unwrap();
        let target = outside.join("target.md");
        let target_content = initial_content(&note.key, &note.hash);
        fs::write(&target, &target_content).unwrap();
        fs::remove_file(&note.path).unwrap();
        symlink(&target, &note.path).unwrap();

        let err = append_note(&config, "src/main.rs", "append").unwrap_err();

        assert!(err.contains("failed to open"), "{err}");
        assert_eq!(fs::read_to_string(&target).unwrap(), target_content);
    });
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
