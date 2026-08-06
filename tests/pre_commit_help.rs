use std::process::Command;

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

#[test] // xpec: D8
fn pre_commit_install_help_lists_action_usage() {
    let output = canon()
        .args(["pre-commit", "install", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon pre-commit install"));
    assert!(stdout.contains("Install the canon pre-commit hook"));
}
