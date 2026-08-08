use crate::git::{git_config_get, GitConfigGetError};
use crate::logs::error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use std::path::Path;

const LOG_MAX_SIZE_CONFIG_KEY: &str = "canon.logs.maxSize";
const DEFAULT_LOG_MAX_SIZE_CONFIG_VALUE: &str = "0M";
pub(crate) const ACTIVE_DIAGNOSTIC_LOG_FILE: &str = "0.jsonl";
// [fh,Yq] Rotation uses this fixed file set, while `max_bytes` bounds their
// combined content.
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
    // [kK,fh,hr,Yq] Command code always constructs applicable runtime events
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
    // xpec: Yq
    (config.max_bytes / file_count as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::DiagnosticLogWriter;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: kK,hr,Yq
    fn zero_limit_renders_events_without_persisting_runtime_logs() {
        let root = git_temp_root("diagnostic-logs-disabled");
        let state_root = crate::state_paths::CanonStateRoot::resolve(&root).unwrap();
        let mut writer =
            DiagnosticLogWriter::create_from_plan(&root, DiagnosticLogPlan::prepare(&root))
                .unwrap();

        writer.emit_event("info", "test.event", &[]).unwrap();
        let error = writer
            .emit_event("invalid\nlevel", "test.event", &[])
            .unwrap_err();

        assert!(error.to_string().contains("not a single-line label"));
        assert!(!state_root.join("logs/0.jsonl").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 2g,g2,Yq
    fn writer_uses_canon_git_configuration() {
        let root = git_temp_root("configured-diagnostic-logs");
        let configured = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["config", "canon.logs.maxSize", "8M"])
            .output()
            .unwrap();
        assert!(configured.status.success());
        let state_root = crate::state_paths::CanonStateRoot::resolve(&root).unwrap();

        let mut writer = DiagnosticLogWriter::create(&root).unwrap();
        writer.emit_event("info", "test.event", &[]).unwrap();

        assert!(state_root.join("logs/0.jsonl").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 2g,gO
    fn configured_log_size_does_not_ignore_value_whitespace() {
        let root = git_temp_root("diagnostic-log-whitespace");
        for value in [" 8M", "8M "] {
            let configured = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", "canon.logs.maxSize", value])
                .output()
                .unwrap();
            assert!(configured.status.success());
            assert!(DiagnosticLogPlan::prepare(&root).into_config().is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn git_temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("canon-test-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .unwrap();
        // xpec: kK,hr,Yq
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }
}
