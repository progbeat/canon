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
    assert!(stdout.contains("show"));
}

#[test]
fn check_help_lists_public_options() {
    let output = canon().args(["check", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon check"));
    assert!(stdout.contains("--ignore-cooldown"));
    assert!(stdout.contains("--keep-going"));
    assert!(stdout.contains("--preset"));
}

#[test]
fn show_help_lists_public_options() {
    let output = canon().args(["show", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon show"));
    assert!(stdout.contains("[SELECTOR]"));
    assert!(stdout.contains("[PATHSPEC]"));
    assert!(stdout.contains("--tree"));
}
