use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

// This mirrors the template include path used by canon init so the behavior can
// be verified from visible source without reading ignored canon expectation data.
const DEFAULT_CHECK_TEMPLATE_FILE_CONTENTS: &str =
    include_str!("../.canon/templates/default/check.yml");
const DEFAULT_PRE_COMMIT_HOOK_CONTENTS: &str = include_str!("../resources/git-hooks/pre-commit");

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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_path(repo: &Path, path: &str) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", path])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.join(String::from_utf8(output.stdout).unwrap().trim())
}

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
    assert_eq!(
        fs::read_to_string(git_path(&repo, "hooks/pre-commit")).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK_CONTENTS
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

#[test]
fn pre_commit_install_rejects_existing_default_hook() {
    let repo = temp_repo("canon-pre-commit-existing-example");
    init_git_repo(&repo);
    fs::write(repo.join(".git/hooks/pre-commit"), "#!/usr/bin/env sh\n").unwrap();

    let output = canon()
        .args(["pre-commit", "install"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: Can't safely install pre-commit hook.\n\
         ▷ Add `canon gate` manually to the existing pre-commit setup or ask a human to handle it.\n"
    );
}

#[test]
fn check_on_start_hook_confirmation_mismatch_blocks_without_result() {
    let repo = temp_repo("canon-check-on-start-hook");
    init_git_repo(&repo);
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    print: "Type pass:\n"
    confirm: "pass"
    repair-instruction: "Run the blocker fix."
expectations:
  - q: "Does the hook block before evaluator work?"
    a: "yes"
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

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Type pass:\n"));
    assert!(stdout.contains(" 1 blocked, 1 pending in "));
    assert!(stdout.ends_with("Run the blocker fix.\n"));
    assert!(!stdout.contains(" OK\n"));
    assert!(!stdout.contains(" FAILED\n"));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
}

#[test]
fn in_place_check_on_start_hook_confirmation_mismatch_blocks_without_result() {
    let repo = temp_repo("canon-in-place-check-on-start-hook");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    print: "Type pass:\n"
    confirm: "pass"
    repair-instruction: "Run the blocker fix."
expectations:
  - q: "Does the hook block before evaluator work?"
    a: "yes"
"#,
    )
    .unwrap();

    let mut child = canon()
        .args(["check", "--in-place"])
        .current_dir(&repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"fail\n").unwrap();
    let output = child.wait_with_output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Type pass:\n"));
    assert!(stdout.contains(" 1 blocked, 1 pending in "));
    assert!(stdout.ends_with("Run the blocker fix.\n"));
    assert!(!stdout.contains(" OK\n"));
    assert!(!stdout.contains(" FAILED\n"));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
}

#[test]
fn in_place_prohibited_expectation_fields_fail_before_hooks() {
    let repo = temp_repo("canon-in-place-invalid-config-before-hooks");
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(
        repo.join(".canon/check.yml"),
        r#"
version: 1
presets:
  default: {}
hooks:
  on-start:
    print: "This should not print.\n"
expectations:
  - q: "Does in-place reject diff-from before hooks?"
    a: "yes"
    diff-from: :against-tree
"#,
    )
    .unwrap();

    let output = canon()
        .args(["check", "--in-place", "unknown-selector"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: expectation 1 has Git-backed-only config: diff-from\n"
    );
}

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

#[test]
fn check_without_config_renders_documented_recovery_message() {
    let repo = temp_repo("canon-missing-config-example");
    init_git_repo(&repo);

    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
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
    );
}
