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
const ZERO_TOKEN_USAGE_LINE: &str =
    "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n";

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

fn git_path(repo: &Path, path: &str) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", path])
        .current_dir(repo)
        .output()
        .unwrap();
    // xpec: C
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.join(String::from_utf8(output.stdout).unwrap().trim())
}

fn read_only_last_result(xpecs_dir: &Path, file_name: &str) -> serde_json::Value {
    let mut xpec_dirs = fs::read_dir(xpecs_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    xpec_dirs.sort();
    let xpec_dir = match xpec_dirs.as_slice() {
        [xpec_dir] => xpec_dir,
        _ => panic!(
            "expected one persisted xpec directory under {}, found {}",
            xpecs_dir.display(),
            xpec_dirs.len()
        ),
    };
    serde_json::from_str(&fs::read_to_string(xpec_dir.join(file_name)).unwrap()).unwrap()
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

// xpec: C
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

// xpec: 8s
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
    // xpec: 8s
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

    // xpec: 8s
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: 8s
    assert!(stdout.starts_with("Type pass: "));
    // xpec: 8s
    assert!(stdout.contains(". FAIL\nexpected: pass\n"));
    // xpec: AL
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: AL
    assert_eq!(
        stderr,
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let last_fail = read_only_last_result(&repo.join(".git/canon/xpecs"), "last-fail.json");
    // xpec: nv
    assert_eq!(last_fail["status"], "fail");
    // xpec: nv
    assert!(last_fail.get("checkedTreeOid").is_some());
    // xpec: nv
    assert!(last_fail.get("visibleTreeOid").is_none());
    let _ = fs::remove_dir_all(&repo);
}

// xpec: 8s
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

    // xpec: 8s
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: 8s
    assert!(stdout.contains(
        ". FAIL\n│ $ printf \"stdout\\n\"; printf \"stderr\\n\" >&2; printf \"after\\n\"; exit 3\n\
         │ stdout\n│ stderr\n│ after\n"
    ));
    // xpec: 8s
    assert!(stdout.contains("Command exited with code 3 (expected 0).\n"));
    // xpec: AL
    assert!(stdout.contains(" 1 failed in "));
    let stderr = String::from_utf8(output.stderr).unwrap();
    // xpec: AL
    assert_eq!(
        stderr,
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n"
    );
    let last_fail = read_only_last_result(&repo.join("state/xpecs"), "last-fail.json");
    // xpec: nv,Df,eP
    assert_eq!(last_fail["status"], "fail");
    // xpec: nv,Df
    assert!(last_fail.get("qScope").is_none());
    // xpec: nv,Df
    assert!(last_fail.get("visibleScope").is_none());
    // xpec: nv,Df
    assert!(last_fail.get("checkedTreeOid").is_none());
    // xpec: nv,Df
    assert!(last_fail.get("visibleTreeOid").is_none());
    let _ = fs::remove_dir_all(&repo);
}

// xpec: AL,Df
#[test]
fn in_place_prohibited_expectation_fields_fail_before_evaluation() {
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
        // and token-usage contract for invalid in-place config.
        .args(["check", "--in-place", "unknown-selector"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // xpec: AL,Df
    assert!(stdout.contains(" 0 passed in "));
    // xpec: AL,Df
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "{ZERO_TOKEN_USAGE_LINE}Error: expectation 1 has Git-backed-only config: diff-from\n"
        )
    );
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

// xpec: C,AL
#[test]
fn check_without_config_renders_documented_recovery_message() {
    let repo = temp_repo("canon-missing-config-example");
    init_git_repo(&repo);

    let output = canon().arg("check").current_dir(&repo).output().unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(" 0 passed in "));
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
