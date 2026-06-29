use std::process::Command;

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

#[test]
fn hook_install_help_lists_action_usage() {
    let output = canon()
        .args(["hook", "install", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon hook install"));
    assert!(stdout.contains("Install the canon Git hook"));
}
