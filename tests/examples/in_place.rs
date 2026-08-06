use super::*;

// xpec: 1g,90,g2,qc
#[test]
fn non_git_in_place_check_without_state_namespace_stays_memory_backed() {
    let repo = portable_temp_dir("canon-in-place-without-state-namespace");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "Type pass:"
    a: "pass"
"#,
    )
    .unwrap();
    for _ in 0..2 {
        let mut child = canon()
            .args(["check", "--in-place"])
            .current_dir(&repo)
            .env_remove("CANON_STATE_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"pass\n").unwrap();
        let output = child.wait_with_output().unwrap();

        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Type pass:"), "{stdout}");
    }
    let _ = fs::remove_dir_all(&repo);
}

// xpec: H9,90
#[test]
fn fresh_in_place_cli_process_orders_same_rank_by_persisted_fail_history() {
    let repo = portable_temp_dir("canon-in-place-persisted-fail-order");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    rank: 0
    q: "Older answer:"
    a: "pass"
  - to: caller
    rank: 0
    q: "Newer answer:"
    a: "pass"
"#,
    )
    .unwrap();
    let state_dir = repo.join("state");

    let mut first_run = in_place_caller_check(&repo, &state_dir, false);
    first_run
        .stdin
        .take()
        .unwrap()
        .write_all(b"fail\n")
        .unwrap();
    let first_output = first_run.wait_with_output().unwrap();
    assert!(!first_output.status.success(), "{first_output:?}");

    wait_for_later_record_timestamp_second();
    let mut second_run = in_place_caller_check(&repo, &state_dir, true);
    second_run
        .stdin
        .take()
        .unwrap()
        .write_all(b"pass\nfail\n")
        .unwrap();
    let second_output = second_run.wait_with_output().unwrap();
    assert!(!second_output.status.success(), "{second_output:?}");

    // [H9] This is a new process with a fresh invocation-local state cache.
    // Observing the newer prompt first therefore proves the public in-place
    // path reloads persisted fail history before it orders selected work.
    let mut third_run = in_place_caller_check(&repo, &state_dir, true);
    third_run
        .stdin
        .take()
        .unwrap()
        .write_all(b"pass\npass\n")
        .unwrap();
    let third_output = third_run.wait_with_output().unwrap();
    assert!(third_output.status.success(), "{third_output:?}");
    let stdout = String::from_utf8(third_output.stdout).unwrap();
    let newer = stdout.find("Newer answer:").unwrap();
    let older = stdout.find("Older answer:").unwrap();
    assert!(newer < older, "{stdout}");

    let _ = fs::remove_dir_all(&repo);
}

fn in_place_caller_check(
    repo: &std::path::Path,
    state_dir: &std::path::Path,
    keep_going: bool,
) -> std::process::Child {
    let mut command = canon();
    command.args(["check", "--in-place"]);
    if keep_going {
        command.arg("--keep-going");
    }
    command
        .current_dir(repo)
        .env("CANON_STATE_DIR", state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_later_record_timestamp_second() {
    let completed_second = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    while SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        <= completed_second
    {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

// xpec: 1h,w,90
#[test]
fn selected_in_place_prohibited_expectation_fields_fail_before_evaluation() {
    let repo = portable_temp_dir("canon-in-place-invalid-config-before-evaluation");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - q: "Does in-place reject diff-from before evaluation?"
    a: "yes"
    diff-from: :against-tree
"#,
    )
    .unwrap();

    let output = canon()
        // This public command path covers the `canon check --in-place` output
        // and token-usage contract for invalid in-place config. In-place is
        // explicitly outside default-source feedback eligibility.
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: w,90
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    // xpec: 1h,w,90
    assert_eq!(
        stderr,
        format!(
            "Error: expectation 1 is invalid in in-place mode: \
             `diff-from` requires Git-backed check state\n{ZERO_TOKEN_USAGE_LINE}"
        )
    );
}
