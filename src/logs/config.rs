use crate::git::{git_config_get, GitConfigGetError};
use crate::logs::error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use std::path::Path;

const LOG_MAX_SIZE_CONFIG_KEY: &str = "canon.logs.maxSize";
const DEFAULT_LOG_MAX_SIZE_CONFIG_VALUE: &str = "0M";
pub(crate) const ACTIVE_DIAGNOSTIC_LOG_FILE: &str = "0.jsonl";
const DIAGNOSTIC_LOG_FILES: [&str; 8] = [
    ACTIVE_DIAGNOSTIC_LOG_FILE,
    "1.jsonl",
    "2.jsonl",
    "3.jsonl",
    "4.jsonl",
    "5.jsonl",
    "6.jsonl",
    "7.jsonl",
];

#[derive(Clone, Copy)]
pub(crate) enum DiagnosticLogConfig {
    Disabled,
    Persistent(PersistentDiagnosticLogConfig),
}

pub(crate) struct DiagnosticLogPlan(DiagnosticLogResult<DiagnosticLogConfig>);

impl DiagnosticLogPlan {
    // [B,cg] Capture command-level canon configuration before a check mode is
    // selected. In-place execution consumes this typed value and has no path
    // back to Git configuration or repository metadata.
    pub(crate) fn prepare(command_directory: &Path) -> DiagnosticLogPlan {
        DiagnosticLogPlan(diagnostic_log_config(command_directory))
    }

    pub(crate) fn into_config(self) -> DiagnosticLogResult<DiagnosticLogConfig> {
        self.0
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PersistentDiagnosticLogConfig {
    pub(crate) max_bytes: u64,
}

fn diagnostic_log_config(root: &Path) -> DiagnosticLogResult<DiagnosticLogConfig> {
    diagnostic_log_config_from_value(git_config_get(root, LOG_MAX_SIZE_CONFIG_KEY))
}

fn diagnostic_log_config_from_value(
    configured_value: Result<Option<String>, GitConfigGetError>,
) -> DiagnosticLogResult<DiagnosticLogConfig> {
    let max_bytes = configured_log_max_size(configured_value)?;
    // [7N,13,hr,m8] Command code always constructs applicable runtime events
    // through `DiagnosticLogWriter`. This configuration controls persistent
    // storage: zero stores none, while a positive bound uses bounded JSONL.
    Ok(if max_bytes == 0 {
        DiagnosticLogConfig::Disabled
    } else {
        DiagnosticLogConfig::Persistent(PersistentDiagnosticLogConfig { max_bytes })
    })
}

fn configured_log_max_size(
    configured_value: Result<Option<String>, GitConfigGetError>,
) -> DiagnosticLogResult<u64> {
    let value = configured_value
        .map_err(log_config_get_error)?
        .unwrap_or_else(|| DEFAULT_LOG_MAX_SIZE_CONFIG_VALUE.to_string());
    parse_log_max_size(&value)
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
        GitConfigGetError::ReadFailed { status, stderr, .. } => DiagnosticLogError::InvalidConfig {
            key: LOG_MAX_SIZE_CONFIG_KEY,
            reason: format!("could not be read ({}): {}", status, stderr),
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

pub(crate) fn diagnostic_log_files() -> &'static [&'static str] {
    &DIAGNOSTIC_LOG_FILES
}

pub(crate) fn active_log_file_name() -> &'static str {
    ACTIVE_DIAGNOSTIC_LOG_FILE
}

pub(crate) fn active_log_rotation_target_bytes(
    config: &PersistentDiagnosticLogConfig,
    file_count: usize,
) -> u64 {
    // xpec: m8
    (config.max_bytes / file_count as u64).max(1)
}
