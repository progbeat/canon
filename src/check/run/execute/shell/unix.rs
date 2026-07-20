use std::process::Command;

pub(super) fn command(question: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", question]);
    command
}
