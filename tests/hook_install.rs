use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

fn temp_repo() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "canon-hook-install-{}-{unique}",
        std::process::id()
    ))
}

#[test]
fn hook_install_allows_unrelated_default_hook_files() {
    let repo = temp_repo();
    fs::create_dir_all(&repo).unwrap();

    let init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(init.status.success());

    fs::write(repo.join(".git/hooks/post-commit"), "#!/usr/bin/env sh\n").unwrap();

    let output = canon()
        .args(["hook", "install"])
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
        "Installed .git/hooks/pre-commit\n"
    );
}
