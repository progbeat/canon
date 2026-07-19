use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const ZERO_TOKEN_USAGE_LINE: &str =
    "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n";
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

fn portable_temp_dir(prefix: &str) -> PathBuf {
    // [Pi] These integration tests use only portable standard-library path,
    // filesystem, process, and temporary-directory APIs. Assertions describe
    // public CLI behavior and never depend on host path syntax or OS details.
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{unique}-{sequence}"))
}

// xpec: RO,Y8
#[test]
fn init_creates_config_and_refuses_overwrite() {
    let repo = portable_temp_dir("canon-init-example");
    fs::create_dir_all(&repo).unwrap();

    let output = canon().arg("init").current_dir(&repo).output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Created .canon/check.yml\n"
    );
    let created_config = fs::read_to_string(repo.join(".canon/check.yml")).unwrap();
    assert!(!created_config.is_empty());

    let output = canon().arg("init").current_dir(&repo).output().unwrap();

    assert_eq!(
        fs::read_to_string(repo.join(".canon/check.yml")).unwrap(),
        created_config
    );

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: .canon/check.yml already exists\n"
    );
}

// xpec: k4,I4,90
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

    // xpec: k4,90
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: k4
    assert!(stdout.starts_with("Type pass: "));
    // xpec: k4,90
    assert!(stdout.contains(". fail\nexpected: pass\n"));
    // xpec: 9b
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: 9b
    assert_eq!(
        stderr,
        "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let _ = fs::remove_dir_all(&repo);
}

// xpec: 1g,I4,g2
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
    assert!(!repo.join("canon").exists());
    assert!(!repo.join("state").exists());
    let _ = fs::remove_dir_all(&repo);
}

// xpec: 1h,9b,I4
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
    // xpec: 9b,I4
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    // xpec: 1h,9b,I4
    assert_eq!(
        stderr,
        format!(
            "Error: expectation 1 is invalid in in-place mode: \
             `diff-from` requires Git-backed check state\n{ZERO_TOKEN_USAGE_LINE}"
        )
    );
}

// xpec: 1h,7N,9b
#[test]
fn invalid_check_arguments_still_emit_the_check_trailer() {
    let repo = portable_temp_dir("canon-invalid-check-argument-trailer");
    fs::create_dir_all(&repo).unwrap();
    let output = canon()
        .args(["check", "--unknown-check-option"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 0 passed in "));
    assert!(!stdout.contains("✓ All checks passed."));
    assert!(!stdout.contains("▷ Run `canon check`"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: 1h,7N,9b
    let diagnostic_offset = stderr.find("unexpected argument").unwrap();
    let trailer_offset = stderr.find(ZERO_TOKEN_USAGE_LINE).unwrap();
    assert!(diagnostic_offset < trailer_offset, "{stderr}");
}

// xpec: Ky
#[test]
fn invalid_ask_arguments_still_emit_token_usage() {
    let repo = portable_temp_dir("canon-invalid-ask-argument-token-usage");
    fs::create_dir_all(&repo).unwrap();
    let output = canon()
        .args(["ask", "Can this pass?", "--keep-going"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE), "{stderr}");
    assert!(stderr.contains("unexpected argument"), "{stderr}");
}

// xpec: 1h,9b,KD
#[test]
fn collected_check_failure_reports_pending_count() {
    let repo = portable_temp_dir("canon-collected-check-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        "presets:\n  default: {}\nexpectations:\n  - to: caller\n    q: \"Answer yes:\"\n    a: \"yes\"\n",
    )
    .unwrap();
    let output = canon()
        .args(["check", "--in-place", "unknown-selector"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    // xpec: 1h,9b
    let diagnostic_offset = stderr.find("expectation").unwrap();
    let trailer_offset = stderr.find(ZERO_TOKEN_USAGE_LINE).unwrap();
    assert!(diagnostic_offset < trailer_offset, "{stderr}");
}

// xpec: 9b
#[test]
fn identity_validation_failure_counts_collected_expectations_as_pending() {
    let repo = portable_temp_dir("canon-collected-identity-failure");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - q: "Duplicate expectation"
    a: "yes"
  - q: "Duplicate expectation"
    a: "yes"
"#,
    )
    .unwrap();
    let output = canon()
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 2 pending in "), "{stdout}\n{stderr}");
    assert!(stderr.contains("duplicate expectation ID"), "{stderr}");
}
