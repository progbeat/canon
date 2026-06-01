use std::process::Command;

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

#[test]
fn top_level_help_lists_public_commands() {
    let output = canon().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon [COMMAND]"));
    assert!(stdout.contains("check"));
    assert!(stdout.contains("gate"));
    assert!(stdout.contains("hook"));
}

#[test]
fn check_help_lists_public_options() {
    let output = canon().args(["check", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon check"));
    assert!(stdout.contains("--ignore-cache"));
    assert!(stdout.contains("--ignore-cooldown"));
    assert!(stdout.contains("--keep-going"));
    assert!(stdout.contains("--preset"));
}
