use crate::git_config::{git_config_get_or_default, GitConfigGetError};
use crate::logging_error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::{DiagnosticLogConfig, DEFAULT_DIAGNOSTIC_LOG_CONFIG};

const LOG_MAX_SIZE_CONFIG_KEY: &str = "canon.logs.maxSize";

pub(crate) fn diagnostic_log_config(
    root: &std::path::Path,
) -> DiagnosticLogResult<DiagnosticLogConfig> {
    Ok(DiagnosticLogConfig {
        max_bytes: configured_log_max_size(root)?,
        files: DEFAULT_DIAGNOSTIC_LOG_CONFIG.files,
    })
}

fn configured_log_max_size(root: &std::path::Path) -> DiagnosticLogResult<u64> {
    git_config_get_or_default(
        root,
        LOG_MAX_SIZE_CONFIG_KEY,
        DEFAULT_DIAGNOSTIC_LOG_CONFIG.max_bytes,
        parse_log_max_size,
        log_config_get_error,
    )
}

fn log_config_get_error(err: GitConfigGetError) -> DiagnosticLogError {
    match err {
        GitConfigGetError::Command(source) => DiagnosticLogError::Command {
            command: "git config",
            source,
        },
        GitConfigGetError::InvalidOutput { stream, message } => {
            let action = match stream {
                "stdout" => "read git config stdout",
                "stderr" => "read git config stderr",
                _ => "read git config output",
            };
            external_log_error(action, message)
        }
        GitConfigGetError::ReadFailed { stderr, .. } => DiagnosticLogError::InvalidConfig {
            key: LOG_MAX_SIZE_CONFIG_KEY,
            reason: format!("could not be read: {}", stderr),
        },
    }
}

fn parse_log_max_size(value: &str) -> DiagnosticLogResult<u64> {
    if value.is_empty() {
        return Err(invalid_log_config("must not be empty"));
    }
    let invalid_byte_count =
        || invalid_log_config("must be a byte count with optional M or G suffix");
    let (digits, multiplier) = match value.as_bytes().last().copied() {
        Some(b'M') => (&value[..value.len() - 1], 1024 * 1024),
        Some(b'G') => (&value[..value.len() - 1], 1024 * 1024 * 1024),
        Some(byte) if byte.is_ascii_digit() => (value, 1),
        _ => return Err(invalid_byte_count()),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_byte_count());
    }
    let value = digits
        .parse::<u64>()
        .map_err(|_| invalid_log_config("value is too large"))?;
    value
        .checked_mul(multiplier)
        .ok_or_else(|| invalid_log_config("value is too large"))
}

fn invalid_log_config(reason: impl Into<String>) -> DiagnosticLogError {
    DiagnosticLogError::InvalidConfig {
        key: LOG_MAX_SIZE_CONFIG_KEY,
        reason: reason.into(),
    }
}

pub(crate) fn diagnostic_log_files(config: &DiagnosticLogConfig) -> DiagnosticLogResult<&[&str]> {
    if config.files.is_empty() {
        return Err(DiagnosticLogError::EmptyFileList);
    }
    Ok(config.files)
}

pub(crate) fn active_log_file_name(config: &DiagnosticLogConfig) -> DiagnosticLogResult<&str> {
    Ok(diagnostic_log_files(config)?[0])
}

pub(crate) fn active_log_max_bytes(config: &DiagnosticLogConfig, file_count: usize) -> u64 {
    if config.max_bytes == 0 {
        return u64::MAX;
    }
    (config.max_bytes / file_count as u64).max(1)
}
