use super::*;

// xpec: qc,90
#[test]
fn caller_xpec_wrong_answer_fails_with_expected_answer() {
    let repo = portable_temp_dir("canon-check-caller-xpec");
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
    let mut child = canon()
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .env("CANON_STATE_DIR", repo.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"fail\n").unwrap();
    let output = child.wait_with_output().unwrap();

    // xpec: qc,90
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: qc
    assert!(stdout.starts_with("Type pass: "));
    // xpec: qc,90
    assert!(stdout.contains(". FAIL\nexpected: pass\n"));
    // xpec: w
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: w
    assert_eq!(
        stderr,
        "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let _ = fs::remove_dir_all(&repo);
}

// xpec: qc,90
#[test]
fn caller_end_of_input_is_reported_as_an_evaluation_error() {
    let repo = portable_temp_dir("canon-check-caller-end-of-input");
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

    let output = canon()
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .env("CANON_STATE_DIR", repo.join("state"))
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Type pass: "), "{stdout}");
    assert!(
        stdout.contains(". FAIL\nerror: failed to read caller answer: end of input\n"),
        "{stdout}"
    );
    assert!(stdout.contains(" 1 failed in "), "{stdout}");
}

// xpec: w
#[test]
fn check_stops_after_failure_unless_keep_going_is_requested() {
    let repo = portable_temp_dir("canon-check-stop-after-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: caller
    q: "First answer:"
    a: "yes"
    rank: 0
  - to: caller
    q: "Second answer:"
    a: "yes"
    rank: 1
"#,
    )
    .unwrap();

    let default_output = run_caller_check(&repo, false, b"no\nyes\n", "state-default");
    assert!(!default_output.status.success());
    let default_stdout = String::from_utf8(default_output.stdout).unwrap();
    assert!(
        default_stdout.contains("First answer: "),
        "{default_stdout}"
    );
    assert!(
        !default_stdout.contains("Second answer: "),
        "{default_stdout}"
    );
    assert!(
        default_stdout.contains(" 1 failed, 1 pending in "),
        "{default_stdout}"
    );

    let keep_going_output = run_caller_check(&repo, true, b"no\nyes\n", "state-keep-going");
    assert!(!keep_going_output.status.success());
    let keep_going_stdout = String::from_utf8(keep_going_output.stdout).unwrap();
    assert!(
        keep_going_stdout.contains("First answer: "),
        "{keep_going_stdout}"
    );
    assert!(
        keep_going_stdout.contains("Second answer: "),
        "{keep_going_stdout}"
    );
    assert!(
        keep_going_stdout.contains(" 1 failed, 1 passed in "),
        "{keep_going_stdout}"
    );

    let _ = fs::remove_dir_all(repo);
}

fn run_caller_check(
    repo: &std::path::Path,
    keep_going: bool,
    input: &[u8],
    state_dir: &str,
) -> std::process::Output {
    let mut command = canon();
    command.args(["check", "--in-place"]);
    if keep_going {
        command.arg("--keep-going");
    }
    let mut child = command
        .current_dir(repo)
        .env("CANON_STATE_DIR", repo.join(state_dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
