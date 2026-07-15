use crate::check::CheckRecord;
use crate::fs_util::ensure_dir_without_symlinks;
use crate::logs::config::{
    active_log_file_name, active_log_max_bytes, diagnostic_log_files,
    diagnostic_logs_explicitly_disabled, DiagnosticLogConfig,
};
use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::logs::lock::acquire_diagnostic_log_lock;
use crate::logs::render::render_runtime_log_event;
use crate::logs::rotation::{
    active_log_size, append_runtime_log_event_to_file, open_runtime_log_file,
    rotate_active_diagnostic_logs, rotate_active_diagnostic_logs_to_fit,
    rotate_diagnostic_logs_with_config,
};
use crate::repo_inspection::RepoInspectionCache;
use crate::state_paths::canon_state_path;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) use crate::logs::config::diagnostic_log_config;
pub(crate) use crate::logs::error::DiagnosticLogError;

pub(crate) struct DiagnosticLogWriter {
    path: PathBuf,
    log_dir: PathBuf,
    config: DiagnosticLogConfig,
}

pub(crate) enum DiagnosticRecordEvent {
    Expectation,
    Interrogation,
}

impl DiagnosticRecordEvent {
    fn result_event(&self) -> &'static str {
        match self {
            DiagnosticRecordEvent::Expectation => "expectation.result",
            DiagnosticRecordEvent::Interrogation => "interrogation.result",
        }
    }

    fn review_event(&self) -> &'static str {
        match self {
            DiagnosticRecordEvent::Expectation => "expectation.review_required",
            DiagnosticRecordEvent::Interrogation => "interrogation.review_required",
        }
    }
}

impl DiagnosticLogWriter {
    // Runtime-log ownership is intentionally centralized here: config resolves
    // `${CANON_STATE_DIR}/logs/0.jsonl`, rotation keeps older JSONL files in
    // that directory, and every `write_event` call renders, appends, flushes,
    // and rotates one complete runtime-log object. `logs::render` validates the
    // common fields and known event schemas, while `logs::events`,
    // `check::interrogation::session`, and
    // `check::interrogation::result::records` route check lifecycle, thread
    // lifecycle/restart, agent request/response/failure, token-usage, cache,
    // record and query-result events through this writer.
    #[cfg(test)]
    pub(crate) fn create(root: &Path) -> DiagnosticLogResult<DiagnosticLogWriter> {
        let mut cache = RepoInspectionCache::new();
        DiagnosticLogWriter::create_with_cache(root, &mut cache)
    }

    pub(crate) fn create_with_cache(
        root: &Path,
        cache: &mut RepoInspectionCache,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        let prepared = prepare_diagnostic_log(root, cache)?;
        if !diagnostic_logs_explicitly_disabled(&prepared.config) {
            let _lock = acquire_diagnostic_log_lock(&prepared.log_dir)?;
            rotate_diagnostic_logs_with_config(&prepared.log_dir, &prepared.config)?;
        }
        Ok(DiagnosticLogWriter {
            path: prepared.path,
            log_dir: prepared.log_dir,
            config: prepared.config,
        })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn write_record_event(
        &mut self,
        event: DiagnosticRecordEvent,
        record: &CheckRecord,
    ) -> DiagnosticLogResult<()> {
        let fields = record_log_fields(record);
        self.write_event("info", event.result_event(), &fields)?;
        if let Some(reason) = record.human_review_reason() {
            let mut review_fields = fields;
            review_fields.push(("reason", json!(reason)));
            self.write_event("warn", event.review_event(), &review_fields)?;
        }
        Ok(())
    }

    pub(crate) fn write_event(
        &mut self,
        level: &str,
        event: &str,
        fields: &[(&str, Value)],
    ) -> DiagnosticLogResult<()> {
        write_runtime_log_event_with_rotation(
            &self.log_dir,
            &self.path,
            &self.config,
            level,
            event,
            fields,
        )
    }
}

fn record_log_fields(record: &CheckRecord) -> Vec<(&'static str, Value)> {
    vec![
        ("id", json!(record.id)),
        ("observed", json!(record.observed)),
        ("evidence", json!(record.evidence)),
        ("scope", json!(record.scope)),
        ("prompt", json!(record.question_text())),
        ("expected", json!(record.expected_answer_text())),
    ]
}

struct PreparedDiagnosticLog {
    log_dir: PathBuf,
    path: PathBuf,
    config: DiagnosticLogConfig,
}

fn prepare_diagnostic_log(
    root: &Path,
    cache: &mut RepoInspectionCache,
) -> DiagnosticLogResult<PreparedDiagnosticLog> {
    let config = diagnostic_log_config(root)?;
    prepare_diagnostic_log_with_config(root, cache, config)
}

fn prepare_diagnostic_log_with_config(
    root: &Path,
    cache: &mut RepoInspectionCache,
    config: DiagnosticLogConfig,
) -> DiagnosticLogResult<PreparedDiagnosticLog> {
    if diagnostic_logs_explicitly_disabled(&config) {
        return disabled_diagnostic_log(root, config);
    }
    let log_dir = diagnostic_log_dir(root, cache)?;
    ensure_dir_without_symlinks(&log_dir)
        .map_err(|message| external_log_error("create diagnostic log directory", message))?;
    let path = log_dir.join(active_log_file_name());
    Ok(PreparedDiagnosticLog {
        log_dir,
        path,
        config,
    })
}

fn diagnostic_log_dir(
    root: &Path,
    _cache: &mut RepoInspectionCache,
) -> DiagnosticLogResult<PathBuf> {
    canon_state_path(root, "logs")
        .map_err(|message| external_log_error("resolve diagnostic log directory", message))
}

fn disabled_diagnostic_log(
    root: &Path,
    config: DiagnosticLogConfig,
) -> DiagnosticLogResult<PreparedDiagnosticLog> {
    debug_assert!(diagnostic_logs_explicitly_disabled(&config));
    let log_dir = canon_state_path(root, "logs")
        .map_err(|message| external_log_error("resolve diagnostic log directory", message))?;
    let path = log_dir.join(active_log_file_name());
    Ok(PreparedDiagnosticLog {
        log_dir,
        path,
        config,
    })
}

fn write_runtime_log_event_with_rotation(
    log_dir: &Path,
    path: &Path,
    config: &DiagnosticLogConfig,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    if diagnostic_logs_explicitly_disabled(config) {
        return Ok(());
    }
    let line = render_runtime_log_event(level, event, fields)?;
    let line_size = line.len() as u64;
    let log_size_limited = config.max_bytes > 0;
    let files = diagnostic_log_files();
    let active_limit = active_log_max_bytes(config, files.len());
    if log_size_limited && line_size > active_limit {
        return Err(DiagnosticLogError::RecordTooLarge {
            size: line_size,
            max_bytes: active_limit,
        });
    }
    let _lock = acquire_diagnostic_log_lock(log_dir)?;
    rotate_diagnostic_logs_with_config(log_dir, config)?;
    if log_size_limited && active_log_size(path)?.saturating_add(line_size) > active_limit {
        rotate_active_diagnostic_logs(log_dir, files)?;
    }
    if log_size_limited {
        rotate_active_diagnostic_logs_to_fit(log_dir, config, line_size)?;
    }
    // Keep file handles local to a single event. A failed write or flush then
    // returns an error without leaving poisoned writer state for the next call.
    let mut file = open_runtime_log_file(path)?;
    append_runtime_log_event_to_file(path, &mut file, &line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        diagnostic_log_files, diagnostic_logs_explicitly_disabled,
        prepare_diagnostic_log_with_config, DiagnosticLogConfig,
    };
    use crate::repo_inspection::RepoInspectionCache;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: Ue
    fn enabled_diagnostic_logs_write_under_local_state_dir_outside_git() {
        let root = temp_root("diagnostic-logs-no-git");
        let mut cache = RepoInspectionCache::new();
        let config = DiagnosticLogConfig {
            max_bytes: 1024,
            explicitly_disabled: false,
        };

        let prepared = prepare_diagnostic_log_with_config(&root, &mut cache, config).unwrap();

        assert!(!diagnostic_logs_explicitly_disabled(&prepared.config));
        let mut writer = super::DiagnosticLogWriter {
            path: prepared.path.clone(),
            log_dir: prepared.log_dir,
            config: prepared.config,
        };
        writer.write_event("info", "test.event", &[]).unwrap();
        assert!(prepared.path.is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: my
    fn diagnostic_logs_rotate_within_configured_log_dir_size() {
        let root = git_temp_root("diagnostic-logs-rotate-within-configured-size");
        let mut cache = RepoInspectionCache::new();
        let config = DiagnosticLogConfig {
            max_bytes: 4000,
            explicitly_disabled: false,
        };
        let prepared = prepare_diagnostic_log_with_config(&root, &mut cache, config).unwrap();
        let mut writer = super::DiagnosticLogWriter {
            path: prepared.path.clone(),
            log_dir: prepared.log_dir.clone(),
            config: prepared.config,
        };

        for index in 0..12 {
            writer
                .write_event(
                    "info",
                    "test.event",
                    &[("index", json!(index)), ("payload", json!("x".repeat(120)))],
                )
                .unwrap();
        }

        assert!(prepared.path.is_file());
        assert!(prepared.log_dir.join("1.jsonl").is_file());
        assert!(
            configured_log_dir_size(&prepared.log_dir, diagnostic_log_files()) <= config.max_bytes
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: my
    fn diagnostic_logs_make_room_for_new_event_with_rotation() {
        let root = git_temp_root("diagnostic-logs-make-room-with-rotation");
        let mut cache = RepoInspectionCache::new();
        let config = DiagnosticLogConfig {
            max_bytes: 1000,
            explicitly_disabled: false,
        };
        let prepared = prepare_diagnostic_log_with_config(&root, &mut cache, config).unwrap();
        let oldest_log_file = diagnostic_log_files().last().unwrap();
        fs::write(prepared.log_dir.join(oldest_log_file), "x".repeat(950)).unwrap();
        let mut writer = super::DiagnosticLogWriter {
            path: prepared.path.clone(),
            log_dir: prepared.log_dir.clone(),
            config: prepared.config,
        };

        writer.write_event("info", "test.event", &[]).unwrap();

        assert!(prepared.path.is_file());
        assert!(
            configured_log_dir_size(&prepared.log_dir, diagnostic_log_files()) <= config.max_bytes
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn git_temp_root(name: &str) -> PathBuf {
        let root = temp_root(name);
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .unwrap_or_else(|err| panic!("failed to run git init: {err}"));
        if !output.status.success() {
            panic!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        root
    }

    fn temp_root(name: &str) -> PathBuf {
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
        root
    }

    fn configured_log_dir_size(log_dir: &Path, files: &[&str]) -> u64 {
        files
            .iter()
            .filter_map(|file| fs::metadata(log_dir.join(file)).ok())
            .map(|metadata| metadata.len())
            .sum()
    }
}
