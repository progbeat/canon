use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

// This mirrors the template include path used by canon init so the behavior can
// be verified from visible source without reading ignored canon expectation data.
const DEFAULT_CHECK_TEMPLATE_FILE_CONTENTS: &str =
    include_str!("../.canon/templates/default/check.yml");
const ZERO_TOKEN_USAGE_LINE: &str =
    "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n";

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

fn temp_repo(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
}

fn init_git_repo(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repo)
        .output()
        .unwrap();
    // xpec: C
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_staged_shell_check_config(repo: &Path) {
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        "presets:\n  default: {}\nexpectations:\n  - to: shell\n    q: \"true\"\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(repo)
        .output()
        .unwrap();
}

// xpec: C
#[test]
fn init_creates_default_template_and_refuses_overwrite() {
    let repo = temp_repo("canon-init-example");
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
    assert_eq!(
        fs::read_to_string(repo.join(".canon/check.yml")).unwrap(),
        DEFAULT_CHECK_TEMPLATE_FILE_CONTENTS
    );

    let output = canon().arg("init").current_dir(&repo).output().unwrap();

    assert_eq!(
        fs::read_to_string(repo.join(".canon/check.yml")).unwrap(),
        DEFAULT_CHECK_TEMPLATE_FILE_CONTENTS
    );

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: .canon/check.yml already exists\n"
    );
}

// xpec: C
#[test]
fn pre_commit_commands_render_documented_messages() {
    let repo = temp_repo("canon-pre-commit-example");
    init_git_repo(&repo);

    let output = canon()
        .args(["pre-commit", "install"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Installed .git/hooks/pre-commit\n"
    );
    let output = canon()
        .args(["pre-commit", "uninstall"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Uninstalled .git/hooks/pre-commit\n"
    );
}

// xpec: nF
#[test]
fn caller_xpec_wrong_answer_fails_with_expected_answer() {
    let repo = temp_repo("canon-check-caller-xpec");
    init_git_repo(&repo);
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
    let add = Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(&repo)
        .output()
        .unwrap();
    // xpec: nF
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let mut child = canon()
        .arg("check")
        .current_dir(&repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"fail\n").unwrap();
    let output = child.wait_with_output().unwrap();

    // xpec: nF
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: nF
    assert!(stdout.starts_with("Type pass: "));
    // xpec: nF
    assert!(stdout.contains(". FAIL\nexpected: pass\n"));
    // xpec: v1
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: v1,NQ
    assert_eq!(
        stderr,
        "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let _ = fs::remove_dir_all(&repo);
}

// xpec: 6
#[test]
fn git_backed_check_accepts_optional_cooldown_field() {
    let repo = temp_repo("canon-check-cooldown-field");
    init_git_repo(&repo);
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: shell
    q: "true"
    cooldown: 1h
"#,
    )
    .unwrap();
    let add = Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains(" 1 passed in "));
    // xpec: NQ
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        ZERO_TOKEN_USAGE_LINE
    );
}

// xpec: nF,Df,nv
#[test]
fn in_place_shell_xpec_reports_transcript_and_exit_code() {
    let repo = temp_repo("canon-in-place-shell-xpec");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
presets:
  default: {}
expectations:
  - to: shell
    q: 'printf "stdout\n"; printf "stderr\n" >&2; printf "after\n"; exit 3'
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
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();

    // xpec: nF
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: nF
    assert!(stdout.contains(
        ". FAIL\n│ $ printf \"stdout\\n\"; printf \"stderr\\n\" >&2; printf \"after\\n\"; exit 3\n\
         │ stdout\n│ stderr\n│ after\n"
    ));
    // xpec: nF
    assert!(stdout.contains("Command exited with code 3 (expected 0).\n"));
    // xpec: v1
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: v1,NQ
    assert_eq!(
        stderr,
        "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let _ = fs::remove_dir_all(&repo);
}

// xpec: v1,Df
#[test]
fn selected_in_place_prohibited_expectation_fields_fail_before_evaluation() {
    let repo = temp_repo("canon-in-place-invalid-config-before-evaluation");
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
        // explicitly outside default-source feedback eligibility, so the
        // self-contained validation error is the only post-summary message.
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: v1,Df
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    // xpec: v1,Df,NQ
    assert_eq!(
        stderr,
        format!(
            "{ZERO_TOKEN_USAGE_LINE}Error: expectation 1 is invalid in in-place mode: \
             `diff-from` requires Git-backed check state\n"
        )
    );
}

// xpec: v1
#[test]
fn invalid_check_arguments_still_emit_the_check_trailer() {
    let repo = temp_repo("canon-invalid-check-argument-trailer");
    init_git_repo(&repo);
    write_staged_shell_check_config(&repo);
    let output = canon()
        .args([
            "check",
            "--config",
            "./.canon/check.yml",
            "--tree",
            ":staged",
            "--against-tree",
            "HEAD",
            "--unknown-check-option",
        ])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 1 pending in "));
    assert!(stdout.contains("▷ Run `canon check` to continue evaluation.\n"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: NQ
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE));
    assert!(stderr.contains("unexpected argument"));
}

// xpec: 0N
#[test]
fn invalid_ask_arguments_still_emit_token_usage() {
    let repo = temp_repo("canon-invalid-ask-argument-token-usage");
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

// xpec: v1,K
#[test]
fn collected_check_failure_reports_pending_feedback() {
    let repo = temp_repo("canon-collected-check-failure");
    init_git_repo(&repo);
    write_staged_shell_check_config(&repo);
    let output = canon()
        .args(["check", "unknown-selector"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 1 pending in "), "{stdout}\n{stderr}");
    assert!(stdout.contains("▷ Run `canon check` to continue evaluation.\n"));
    // xpec: NQ
    assert!(stderr.starts_with(ZERO_TOKEN_USAGE_LINE));
    assert!(stderr.contains("expectation"), "{stderr}");
}

// xpec: NQ
#[test]
fn identity_validation_failure_counts_collected_expectations_as_pending() {
    let repo = temp_repo("canon-collected-identity-failure");
    init_git_repo(&repo);
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
    Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(" 2 pending in "), "{stdout}\n{stderr}");
    assert!(stdout.contains("▷ Run `canon check` to continue evaluation.\n"));
    assert!(stderr.contains("duplicate expectation ID"), "{stderr}");
}

// xpec: C
#[test]
fn gate_rejects_mixed_canon_and_implementation_changes() {
    let repo = temp_repo("canon-gate-example");
    init_git_repo(&repo);

    let output = canon().arg("init").current_dir(&repo).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(repo.join("src-main.py"), "print('hello')\n").unwrap();
    let output = Command::new("git")
        .args(["add", ".canon/check.yml", "src-main.py"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = canon().arg("gate").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "canon gate: .canon/** changes must not be mixed with non-.canon changes\n\
         ▷ Ask human to handle .canon/ changes.\n"
    );
}

// xpec: C
#[test]
fn gate_passes_canon_only_staged_config_deletion() {
    let repo = temp_repo("canon-gate-canon-only-example");
    init_git_repo(&repo);

    let output = canon().arg("init").current_dir(&repo).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .args(["add", ".canon/check.yml"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "init canon",
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_file(repo.join(".canon/check.yml")).unwrap();
    let output = Command::new("git")
        .args(["add", "-u", ".canon/check.yml"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = canon().arg("gate").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}

// xpec: C
#[test]
fn gate_passes_non_canon_staged_change_without_config() {
    let repo = temp_repo("canon-gate-no-config-example");
    init_git_repo(&repo);
    fs::write(repo.join("src-main.py"), "print('hello')\n").unwrap();
    let output = Command::new("git")
        .args(["add", "src-main.py"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = canon().arg("gate").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}

// xpec: C,v1
#[test]
fn check_without_config_renders_documented_recovery_message() {
    let repo = temp_repo("canon-missing-config-example");
    init_git_repo(&repo);

    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 0 passed in "), "{stdout}");
    assert!(stdout.contains("✓ All checks passed. Commit is allowed.\n"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.ends_with(
            "Error: No canon check config found at .canon/check.yml\n\
         ▷ If the human asked you to use any `canon-*` skill in this project, treat \
         the missing config as permission to set up canon rather than asking for \
         confirmation. Run `canon init`, inspect the current project enough to \
         understand its purpose, public behavior, and important invariants, then \
         draft a minimal `.canon/check.yml` that the human can grow over time. Use \
         canon's own `.canon/check.yml` as a reference for structure and style: \
         `https://raw.githubusercontent.com/progbeat/canon/main/.canon/check.yml`. \
         Start with a few simple, objective expectations that protect important \
         user-facing behavior.\n"
        ),
        "{stderr}"
    );
}
