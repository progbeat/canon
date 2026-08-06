use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::logs::config::{
    active_log_file_name, active_log_rotation_target_bytes, diagnostic_log_files,
    PersistentDiagnosticLogConfig,
};
use crate::logs::error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::logs::lock::acquire_ephemeral_diagnostic_log_lock;
use crate::logs::rotation::{
    active_log_size, append_runtime_log_event_to_file, open_runtime_log_file,
    prune_diagnostic_logs_to_fit, rotate_active_diagnostic_logs,
    rotate_diagnostic_logs_with_config,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) struct PersistentRuntimeLogHistory {
    pub(super) path: PathBuf,
    pub(super) log_dir: PathBuf,
    config: PersistentDiagnosticLogConfig,
}

impl PersistentRuntimeLogHistory {
    pub(super) fn activate(&self) -> DiagnosticLogResult<()> {
        // [fh] Every persistent writer activation reapplies the configured
        // whole-directory bound before any event is accepted.
        let _lock = acquire_ephemeral_diagnostic_log_lock(&self.log_dir)?;
        rotate_diagnostic_logs_with_config(&self.log_dir, &self.config)?;
        prune_diagnostic_logs_to_fit(&self.log_dir, &self.config, 0)
    }

    pub(super) fn write_event(&mut self, rendered_event: &str) -> DiagnosticLogResult<()> {
        write_runtime_log_event_with_rotation(
            &self.log_dir,
            &self.path,
            &self.config,
            rendered_event,
        )
    }
}

pub(super) fn disable_persistent_storage_at(
    state_root: Option<&crate::state_paths::CanonStateRoot>,
) -> DiagnosticLogResult<()> {
    let Some(state_root) = state_root else {
        return Ok(());
    };
    let log_dir = state_root.join(crate::state_paths::RUNTIME_LOG_DIR_NAME);
    reject_symlink(&log_dir)
        .map_err(|message| external_log_error("inspect diagnostic log directory", message))?;
    match fs::metadata(&log_dir) {
        Ok(metadata) if metadata.is_dir() => {
            let _lock = acquire_ephemeral_diagnostic_log_lock(&log_dir)?;
            let zero_limit = PersistentDiagnosticLogConfig { max_bytes: 0 };
            prune_diagnostic_logs_to_fit(&log_dir, &zero_limit, 0)
        }
        Ok(_) => Err(external_log_error(
            "inspect diagnostic log directory",
            format!("{} exists but is not a directory", log_dir.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(external_log_error(
            "inspect diagnostic log directory",
            format!("failed to inspect {}: {err}", log_dir.display()),
        )),
    }
}

pub(super) fn prepare_diagnostic_log_backend(
    root: &Path,
    config: PersistentDiagnosticLogConfig,
) -> DiagnosticLogResult<PersistentRuntimeLogHistory> {
    let state_root = crate::state_paths::CanonStateRoot::resolve(root)
        .map_err(|message| external_log_error("resolve diagnostic log directory", message))?;
    prepare_diagnostic_log_backend_at(&state_root, config)
}

pub(super) fn prepare_diagnostic_log_backend_at(
    state_root: &crate::state_paths::CanonStateRoot,
    config: PersistentDiagnosticLogConfig,
) -> DiagnosticLogResult<PersistentRuntimeLogHistory> {
    let log_dir = state_root.join(crate::state_paths::RUNTIME_LOG_DIR_NAME);
    ensure_dir_without_symlinks(&log_dir)
        .map_err(|message| external_log_error("create diagnostic log directory", message))?;
    let path = log_dir.join(active_log_file_name());
    Ok(PersistentRuntimeLogHistory {
        log_dir,
        path,
        config,
    })
}

fn write_runtime_log_event_with_rotation(
    log_dir: &Path,
    path: &Path,
    config: &PersistentDiagnosticLogConfig,
    line: &str,
) -> DiagnosticLogResult<()> {
    let line_size = line.len() as u64;
    let files = diagnostic_log_files();
    let active_rotation_target = active_log_rotation_target_bytes(config, files.len());
    if line_size > config.max_bytes {
        return Err(DiagnosticLogError::RecordTooLarge {
            size: line_size,
            max_bytes: config.max_bytes,
        });
    }
    let _lock = acquire_ephemeral_diagnostic_log_lock(log_dir)?;
    rotate_diagnostic_logs_with_config(log_dir, config)?;
    if active_log_size(path)?.saturating_add(line_size) > active_rotation_target {
        rotate_active_diagnostic_logs(log_dir, files)?;
    }
    // [fh] Every append makes room under the whole-directory byte bound, so
    // rotated files cannot accumulate across repeated runs.
    prune_diagnostic_logs_to_fit(log_dir, config, line_size)?;
    // Keep file handles local to a single event. A failed write or flush then
    // returns an error without leaving poisoned writer state for the next call.
    let mut file = open_runtime_log_file(path)?;
    append_runtime_log_event_to_file(path, &mut file, line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::{DiagnosticLogPlan, DiagnosticLogWriter};
    use serde_json::{json, Value};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: fh,hr
    fn zero_limit_applies_to_existing_runtime_logs() {
        let root = git_temp_root("diagnostic-logs-disabled-zero-limit");
        let log_dir = crate::state_paths::CanonStateRoot::resolve(&root)
            .unwrap()
            .join("logs");
        let mut enabled_writer = writer_with_limit(&root, 8192);
        enabled_writer
            .emit_event("info", "test.event", &[])
            .unwrap();

        let mut writer = writer_with_limit(&root, 0);
        writer.emit_event("info", "test.event", &[]).unwrap();

        assert_eq!(log_dir_size(&log_dir), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: Yq,w
    fn persistent_diagnostic_logs_write_under_canonical_state_dir() {
        let root = git_temp_root("diagnostic-logs-canonical-state");
        let path = crate::state_paths::CanonStateRoot::resolve(&root)
            .unwrap()
            .join("logs/0.jsonl");
        let mut writer = writer_with_limit(&root, 8192);
        writer
            .emit_event("info", "check.start", &[("candidates", json!(["id"]))])
            .unwrap();
        writer.emit_event("info", "test.event", &[]).unwrap();
        let records = read_json_lines(&path);

        // The component contract is the retained runtime-log history. Its
        // short-lived cross-process coordination artifacts are intentionally
        // not exposed as observable path or naming requirements.
        assert!(path.is_file());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["event"], "check.start");
        assert_eq!(records[1]["event"], "test.event");
        assert_eq!(records[0]["processId"], process::id());
        assert!(records
            .iter()
            .all(|record| record["processId"] == records[0]["processId"]));
        assert!(records
            .iter()
            .all(|record| record["invocationId"] == records[0]["invocationId"]));
        assert!(records
            .iter()
            .all(|record| record.get("checkStartedAt").is_none()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: fh
    fn diagnostic_logs_rotate_within_configured_log_dir_size() {
        let root = git_temp_root("diagnostic-logs-rotate-within-configured-size");
        let config = PersistentDiagnosticLogConfig { max_bytes: 4000 };
        let log_dir = crate::state_paths::CanonStateRoot::resolve(&root)
            .unwrap()
            .join("logs");
        let mut writer = writer_with_limit(&root, config.max_bytes);

        for index in 0..12 {
            writer
                .emit_event(
                    "info",
                    "test.event",
                    &[("index", json!(index)), ("payload", json!("x".repeat(120)))],
                )
                .unwrap();
        }

        assert!(log_dir.join("0.jsonl").is_file());
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: fh
    fn persistent_backend_applies_size_bound_before_first_event() {
        let root = git_temp_root("diagnostic-logs-bound-on-create");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1000 };
        let log_dir = crate::state_paths::CanonStateRoot::resolve(&root)
            .unwrap()
            .join("logs");
        let mut writer = writer_with_limit(&root, 4000);
        for index in 0..12 {
            writer
                .emit_event(
                    "info",
                    "test.event",
                    &[("index", json!(index)), ("payload", json!("x".repeat(120)))],
                )
                .unwrap();
        }
        drop(writer);
        let unrelated_path = log_dir.join("other.log");
        fs::write(&unrelated_path, "x".repeat(600)).unwrap();

        let _writer = writer_with_limit(&root, config.max_bytes);

        assert!(unrelated_path.is_file());
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
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
        // xpec: w,fh,hr,Yq
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }

    fn writer_with_limit(root: &Path, max_bytes: u64) -> DiagnosticLogWriter {
        let configured = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "canon.logs.maxSize", &max_bytes.to_string()])
            .output()
            .unwrap();
        // xpec: w,fh,hr,Yq
        assert!(configured.status.success());
        DiagnosticLogWriter::create_from_plan(root, DiagnosticLogPlan::prepare(root)).unwrap()
    }

    fn log_dir_size(log_dir: &Path) -> u64 {
        fs::read_dir(log_dir)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum()
    }

    fn read_json_lines(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
