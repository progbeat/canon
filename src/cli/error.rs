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
    AskFailed(AskFailure),
    CheckFailed,
    GateFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskFailure {
    Query,
    ReviewRequired,
    Output,
    TokenUsage,
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
            CommandError::AskFailed(_) => formatter.write_str("canon ask failed"),
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
    !matches!(
        err,
        CommandError::AskFailed(_) | CommandError::CheckFailed | CommandError::GateFailed
    )
}

fn write_command_error_line(err: &CommandError) -> Result<(), String> {
    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    stderr
        .write_all(render_command_error(err).as_bytes())
        .map_err(|source| format!("failed to write command error to stderr: {}", source))?;
    stderr
        .flush()
        .map_err(|source| format!("failed to flush command error to stderr: {}", source))
}

fn render_command_error(err: &CommandError) -> String {
    let message = err.to_string();
    if is_expectation_diagnostic_block(&message) {
        let mut output = message;
        output.push('\n');
        return output;
    }
    format!("Error: {}\n", message)
}

fn is_expectation_diagnostic_block(message: &str) -> bool {
    message
        .lines()
        .next()
        .is_some_and(|line| line.ends_with(" ERROR") || line.ends_with(" FAILED"))
}

#[cfg(test)]
mod tests {
    use super::{render_command_error, CommandError};

    #[test]
    fn expectation_diagnostic_block_is_not_prefixed_with_generic_error() {
        let rendered = render_command_error(&CommandError::from(
            "x. ERROR\nQuestion?\nError: detail\nEvidence: value".to_string(),
        ));

        assert_eq!(
            rendered,
            "x. ERROR\nQuestion?\nError: detail\nEvidence: value\n"
        );
    }

    #[test]
    fn ordinary_error_keeps_generic_error_prefix() {
        let rendered = render_command_error(&CommandError::from("ordinary failure".to_string()));

        assert_eq!(rendered, "Error: ordinary failure\n");
    }

    #[test] // xpec: 5
    fn ask_failed_has_no_extra_public_diagnostic() {
        assert!(!super::command_error_has_public_diagnostic(
            &CommandError::AskFailed(super::AskFailure::Query)
        ));
    }
}
