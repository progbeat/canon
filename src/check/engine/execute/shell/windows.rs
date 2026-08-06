use std::process::Command;

pub(super) fn command(question: &str) -> Command {
    let mut command = Command::new("cmd.exe");
    command.args(["/D", "/S", "/C", question]);
    command
}
