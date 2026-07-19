use std::process::Command;

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

#[test] // xpec: Y8
fn top_level_help_lists_public_commands() {
    let output = canon().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon [COMMAND]"));
    assert!(stdout.contains("ask"));
    assert!(stdout.contains("check"));
    assert!(stdout.contains("gate"));
    assert!(stdout.contains("pre-commit"));
    assert!(stdout.contains("show"));
}

#[test] // xpec: Y8
fn old_hook_command_is_not_public() {
    let output = canon().arg("hook").output().unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: unknown command: hook\n"
    );
}

#[test] // xpec: 9b
fn check_help_lists_public_options() {
    let output = canon().args(["check", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon check"));
    assert!(stdout.contains("--keep-going"));
    assert!(stdout.contains("--no-sandbox"));
    assert!(stdout.contains("Disable canon-managed sandboxing"));
    assert!(stdout.contains("--in-place"));
    assert!(stdout.contains("not:<ID-PREFIX>"));
    assert!(stdout.contains("canon check not:a7F not:K9m"));
    assert!(!stdout.contains("-q"));
    assert!(!stdout.contains("--preset"));
    assert!(!stdout.contains("--scope"));
    assert!(!stdout.contains("--ignore-cooldown"));
    assert!(!stdout.contains("--all"));
}

#[test] // xpec: 3i5,Ky
fn ask_help_lists_public_options() {
    let output = canon().args(["ask", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon ask"));
    assert!(stdout.contains("<QUESTION>"));
    assert!(!stdout.contains("--scope"));
    assert!(stdout.contains("--preset"));
    assert!(stdout.contains("--tree"));
    assert!(stdout.contains("--against-tree"));
    assert!(stdout.contains("--in-place"));
    assert!(!stdout.contains("--no-sandbox"));
    assert!(stdout.contains("canon ask \"Does the app expose Undo?\""));
    assert!(!stdout.contains("--keep-going"));
    assert!(!stdout.contains("not:<ID-PREFIX>"));
}

#[test] // xpec: E
fn show_help_lists_public_options() {
    let output = canon().args(["show", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: canon show"));
    assert!(stdout.contains("[SELECTOR]"));
    assert!(stdout.contains("[PATHSPEC]"));
    assert!(stdout.contains("--tree"));
    assert!(stdout.contains("not:<ID-PREFIX>"));
}
