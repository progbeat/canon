use crate::logs::DiagnosticLogError;
use std::borrow::Cow;
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandError {
    Message(Cow<'static, str>),
    InitDoesNotAcceptArguments,
    PwdDoesNotAcceptArguments,
    UnknownOption(String),
    UnknownCommand(String),
    CheckFailed,
    GateFailed,
}

impl From<String> for CommandError {
    fn from(message: String) -> CommandError {
        CommandError::Message(Cow::Owned(message))
    }
}

impl From<DiagnosticLogError> for CommandError {
    fn from(err: DiagnosticLogError) -> CommandError {
        CommandError::Message(Cow::Owned(err.to_string()))
    }
}

impl From<&'static str> for CommandError {
    fn from(message: &'static str) -> CommandError {
        CommandError::Message(Cow::Borrowed(message))
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Message(message) => formatter.write_str(message),
            CommandError::InitDoesNotAcceptArguments => {
                formatter.write_str("init does not accept arguments")
            }
            CommandError::PwdDoesNotAcceptArguments => {
                formatter.write_str("pwd does not accept arguments")
            }
            CommandError::UnknownOption(option) => write!(formatter, "unknown option: {option}"),
            CommandError::UnknownCommand(command) => {
                write!(formatter, "unknown command: {command}")
            }
            CommandError::CheckFailed => formatter.write_str("canon check failed"),
            CommandError::GateFailed => formatter.write_str("canon gate failed"),
        }
    }
}

pub(super) fn report_command_error(err: CommandError) -> CommandError {
    if command_error_has_public_diagnostic(&err) {
        let _ = write_command_error_line(&err);
    }
    err
}

fn command_error_has_public_diagnostic(err: &CommandError) -> bool {
    // These commands already wrote their public diagnostics before returning a
    // sentinel error for the process exit status.
    !matches!(err, CommandError::CheckFailed | CommandError::GateFailed)
}

fn write_command_error_line(err: &CommandError) -> Result<(), String> {
    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    writeln!(stderr, "Error: {}", err)
        .map_err(|source| format!("failed to write command error to stderr: {}", source))?;
    stderr
        .flush()
        .map_err(|source| format!("failed to flush command error to stderr: {}", source))
}
