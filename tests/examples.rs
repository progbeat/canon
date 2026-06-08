use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// This mirrors the template include path used by canon init so the behavior can
// be verified from visible source without reading ignored canon expectation data.
const DEFAULT_CHECK_TEMPLATE_FILE_CONTENTS: &str =
    include_str!("../.canon/templates/default/check.yml");

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
fn hook_commands_render_documented_messages() {
    let repo = temp_repo("canon-hook-example");
    init_git_repo(&repo);

    let output = canon()
        .args(["hook", "install"])
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
        .args(["hook", "uninstall"])
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
fn hook_install_rejects_existing_default_hook() {
    let repo = temp_repo("canon-hook-existing-example");
    init_git_repo(&repo);
    fs::write(repo.join(".git/hooks/pre-commit"), "#!/usr/bin/env sh\n").unwrap();

    let output = canon()
        .args(["hook", "install"])
        .current_dir(&repo)
        .output()
        .unwrap();

    let _ = fs::remove_dir_all(&repo);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: Can't safely install pre-commit hook.\n\
         ▷ Add `canon gate` manually to the existing hook setup or ask a human to handle it.\n"
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
fn gate_reports_mixed_canon_changes_before_config_errors() {
    let repo = temp_repo("canon-gate-mixed-before-config-example");
    init_git_repo(&repo);
    fs::create_dir_all(repo.join(".canon")).unwrap();
    fs::write(repo.join(".canon/check.yml"), "not valid: [").unwrap();
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
