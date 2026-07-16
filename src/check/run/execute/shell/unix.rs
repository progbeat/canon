use std::process::Command;

pub(super) fn command(question: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    // The wrapper gives the command one shared stdout/stderr stream, preserving
    // the transcript order produced by the command while keeping the question
    // itself a separate argument with no quoting reconstruction.
    command.args(["-c", "exec /bin/sh -c \"$1\" 2>&1", "canon-shell", question]);
    command
}
