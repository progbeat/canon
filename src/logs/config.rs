use crate::git::config::{git_config_get, git_config_get_or_else, GitConfigGetError};
use crate::logs::error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::{DiagnosticLogConfig, DEFAULT_DIAGNOSTIC_LOG_CONFIG};
use std::path::Path;

const LOG_MAX_SIZE_CONFIG_KEY: &str = "canon.logs.maxSize";
const DEFAULT_LOG_MAX_SIZE_CONFIG_VALUE: &str = "0M";
const THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY: &str =
    "canon.threadReuse.carryoverTokenTarget";

pub(crate) fn diagnostic_log_config(root: &Path) -> DiagnosticLogResult<DiagnosticLogConfig> {
    let (max_bytes, explicitly_disabled) = configured_log_max_size(root)?;
    Ok(DiagnosticLogConfig {
        max_bytes,
        explicitly_disabled,
        files: DEFAULT_DIAGNOSTIC_LOG_CONFIG.files,
    })
}

fn configured_log_max_size(root: &Path) -> DiagnosticLogResult<(u64, bool)> {
    let value = git_config_get(root, LOG_MAX_SIZE_CONFIG_KEY)
        .map_err(log_config_get_error)?
        .unwrap_or_else(|| DEFAULT_LOG_MAX_SIZE_CONFIG_VALUE.to_string());
    let max_bytes = parse_log_max_size(&value)?;
    Ok((max_bytes, max_bytes == 0))
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

pub(crate) fn diagnostic_logs_unlimited(config: &DiagnosticLogConfig) -> bool {
    !config.explicitly_disabled && config.max_bytes == 0
}

pub(crate) fn diagnostic_logs_explicitly_disabled(config: &DiagnosticLogConfig) -> bool {
    config.explicitly_disabled
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CarryoverTokenTarget {
    pub(crate) min: u64,
    pub(crate) max: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThreadReuseConfig {
    pub(crate) carryover_token_target: CarryoverTokenTarget,
}

pub(crate) const DEFAULT_THREAD_REUSE_CONFIG: ThreadReuseConfig = ThreadReuseConfig {
    carryover_token_target: CarryoverTokenTarget {
        min: 10_000,
        max: 30_000,
    },
};

pub(crate) fn thread_reuse_config(root: &Path) -> Result<ThreadReuseConfig, String> {
    Ok(ThreadReuseConfig {
        carryover_token_target: configured_carryover_token_target(root)?,
    })
}

fn configured_carryover_token_target(root: &Path) -> Result<CarryoverTokenTarget, String> {
    git_config_get_or_else(
        root,
        THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY,
        || DEFAULT_THREAD_REUSE_CONFIG.carryover_token_target,
        parse_carryover_token_target,
        thread_reuse_git_config_error,
    )
}

fn thread_reuse_git_config_error(err: GitConfigGetError) -> String {
    match err {
        GitConfigGetError::Command(err) => format!("failed to run git config: {}", err),
        GitConfigGetError::InvalidOutput { message, .. } => message,
        GitConfigGetError::ReadFailed {
            key,
            status,
            stderr,
        } => {
            format!("{} could not be read ({}): {}", key, status, stderr)
        }
    }
}

pub(crate) fn parse_carryover_token_target(value: &str) -> Result<CarryoverTokenTarget, String> {
    let (min, max) = value
        .split_once(',')
        .filter(|(min, max)| !min.is_empty() && !max.is_empty())
        .ok_or_else(|| invalid_carryover_token_target("must be a MIN,MAX token range"))?;
    let min = parse_positive_token_count(min)?;
    let max = parse_positive_token_count(max)?;
    if min > max {
        return Err(invalid_carryover_token_target(
            "MIN must be less than or equal to MAX",
        ));
    }
    Ok(CarryoverTokenTarget { min, max })
}

fn parse_positive_token_count(value: &str) -> Result<u64, String> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_carryover_token_target(
            "MIN and MAX must be positive integers",
        ));
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| invalid_carryover_token_target("value is too large"))?;
    if parsed == 0 {
        return Err(invalid_carryover_token_target(
            "MIN and MAX must be greater than zero",
        ));
    }
    Ok(parsed)
}

fn invalid_carryover_token_target(reason: &str) -> String {
    format!(
        "{} {}",
        THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY, reason
    )
}
