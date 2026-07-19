use crate::check::CheckRecord;
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::logs::config::{
    active_log_file_name, active_log_rotation_target_bytes, diagnostic_log_files,
    DiagnosticLogConfig, DiagnosticLogPlan, PersistentDiagnosticLogConfig,
};
use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::logs::lock::acquire_diagnostic_log_lock;
use crate::logs::render::render_runtime_log_process_event;
use crate::logs::rotation::{
    active_log_size, append_runtime_log_event_to_file, open_runtime_log_file,
    prune_diagnostic_logs_to_fit, rotate_active_diagnostic_logs,
    rotate_diagnostic_logs_with_config,
};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use crate::logs::error::DiagnosticLogError;

/// Unconditional command-facing runtime-event entry point.
///
/// Commands construct every applicable event through this API. Configuration
/// renders and validates every event, retains it in invocation memory, and
/// independently attempts to append it to bounded cross-invocation JSONL
/// history under `CANON_STATE_DIR`.
/// No command call site branches on the selected storage.
pub(crate) struct DiagnosticLogWriter {
    invocation_events: Vec<String>,
    persistent_history: Option<PersistentRuntimeLogHistory>,
    invocation_id: String,
    deferred_write_error: Option<String>,
    defers_write_errors: bool,
}

struct PersistentRuntimeLogHistory {
    path: PathBuf,
    log_dir: PathBuf,
    config: PersistentDiagnosticLogConfig,
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
    // Runtime-event ownership is intentionally centralized here. Every
    // `emit_event` call renders and validates one complete event and retains
    // its JSONL representation in invocation memory. The persistence layer
    // separately attempts to append it under `${CANON_STATE_DIR}/logs`.
    // `logs::events`,
    // `check::interrogation::session`, and
    // `check::interrogation::result::records` route check lifecycle, thread
    // lifecycle/restart, agent request/response/failure, token-usage, cache,
    // record and query-result events through this writer.
    pub(crate) fn create_from_plan(
        root: &Path,
        plan: DiagnosticLogPlan,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        let config = plan.into_config()?;
        Self::create_with_config(root, config)
    }

    pub(crate) fn create_in_place(
        plan: DiagnosticLogPlan,
        persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        // [B,cg] The mode receives an already resolved command control-plane
        // value. It cannot read checked files, Git objects, refs, diffs,
        // scopes, tracking state, file hiding, or evaluator context from it.
        let config = plan.into_config()?;
        Self::create_with_explicit_persistent_state(persistent_state_root, config)
    }

    #[cfg(test)]
    pub(crate) fn create(root: &Path) -> DiagnosticLogResult<DiagnosticLogWriter> {
        Self::create_from_plan(root, DiagnosticLogPlan::prepare(root))
    }

    fn create_with_config(
        root: &Path,
        config: DiagnosticLogConfig,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        match config {
            DiagnosticLogConfig::Disabled => {
                let state_root = crate::state_paths::CanonStateRoot::resolve_if_available(root)
                    .map_err(|message| {
                        external_log_error("resolve diagnostic log directory", message)
                    })?;
                Self::without_persistent_storage_at(state_root.as_ref())
            }
            DiagnosticLogConfig::Persistent(config) => {
                let backend = prepare_diagnostic_log_backend(root, config)?;
                Self::with_persistent_backend(backend)
            }
        }
    }

    fn create_with_explicit_persistent_state(
        persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
        config: DiagnosticLogConfig,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        // [g2] An explicit state namespace makes JSONL records intentional
        // cross-invocation history. Without one, current events stay in memory.
        match (config, persistent_state_root) {
            (DiagnosticLogConfig::Persistent(config), Some(persistent_state_root)) => {
                let backend = prepare_diagnostic_log_backend_at(persistent_state_root, config)?;
                Self::with_persistent_backend(backend)
            }
            (DiagnosticLogConfig::Disabled, persistent_state_root) => {
                Self::without_persistent_storage_at(persistent_state_root)
            }
            (DiagnosticLogConfig::Persistent(_), None) => Self::without_persistent_storage(),
        }
    }

    fn without_persistent_storage() -> DiagnosticLogResult<DiagnosticLogWriter> {
        Self::with_optional_persistent_history(None)
    }

    fn with_optional_persistent_history(
        persistent_history: Option<PersistentRuntimeLogHistory>,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        Ok(DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history,
            invocation_id: allocate_runtime_log_invocation_id()?,
            deferred_write_error: None,
            defers_write_errors: false,
        })
    }

    fn without_persistent_storage_at(
        state_root: Option<&crate::state_paths::CanonStateRoot>,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        if let Some(state_root) = state_root {
            let log_dir = state_root.join("logs");
            reject_symlink(&log_dir).map_err(|message| {
                external_log_error("inspect diagnostic log directory", message)
            })?;
            match fs::metadata(&log_dir) {
                Ok(metadata) if metadata.is_dir() => {
                    let _lock = acquire_diagnostic_log_lock(&log_dir)?;
                    let zero_limit = PersistentDiagnosticLogConfig { max_bytes: 0 };
                    prune_diagnostic_logs_to_fit(&log_dir, &zero_limit, 0)?;
                }
                Ok(_) => {
                    return Err(external_log_error(
                        "inspect diagnostic log directory",
                        format!("{} exists but is not a directory", log_dir.display()),
                    ));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(external_log_error(
                        "inspect diagnostic log directory",
                        format!("failed to inspect {}: {err}", log_dir.display()),
                    ));
                }
            }
        }
        Self::without_persistent_storage()
    }

    fn with_persistent_backend(
        backend: PersistentRuntimeLogHistory,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        {
            let _lock = acquire_diagnostic_log_lock(&backend.log_dir)?;
            rotate_diagnostic_logs_with_config(&backend.log_dir, &backend.config)?;
            prune_diagnostic_logs_to_fit(&backend.log_dir, &backend.config, 0)?;
        }
        Self::with_optional_persistent_history(Some(backend))
    }

    /// Keeps runtime observability failures from interrupting the operation
    /// whose events are being recorded. Event writes are still attempted at
    /// every call site; the first failure is returned by
    /// `finish_deferred_writes` after the operation's required effects.
    pub(crate) fn defer_write_errors(&mut self) {
        debug_assert!(!self.defers_write_errors);
        debug_assert!(self.deferred_write_error.is_none());
        self.defers_write_errors = true;
    }

    pub(crate) fn finish_deferred_writes(&mut self) -> Result<(), String> {
        debug_assert!(self.defers_write_errors);
        self.defers_write_errors = false;
        match self.deferred_write_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn write_record_event(
        &mut self,
        event: DiagnosticRecordEvent,
        record: &CheckRecord,
    ) -> DiagnosticLogResult<()> {
        let fields = record_log_fields(record);
        self.emit_event("info", event.result_event(), &fields)?;
        if let Some(reason) = record.human_review_reason() {
            let mut review_fields = fields;
            review_fields.push(("reason", json!(reason)));
            self.emit_event("warn", event.review_event(), &review_fields)?;
        }
        Ok(())
    }

    pub(crate) fn emit_event(
        &mut self,
        level: &str,
        event: &str,
        fields: &[(&str, Value)],
    ) -> DiagnosticLogResult<()> {
        // [7N,hJ,hr] Rendering and validating the complete runtime event is
        // unconditional. Storage policy is an internal concern and controls
        // only whether that event receives a persistent JSONL representation.
        let rendered = render_runtime_log_process_event(&self.invocation_id, level, event, fields)?;
        // [g2,hJ,R1] Every valid event and its primary invocation correlation
        // ID remain invocation-local. Persistent JSONL history, when
        // configured, receives a separate copy.
        self.invocation_events.push(rendered.line.clone());
        let result = match self.persistent_history.as_mut() {
            Some(history) => history.write_event(&rendered.line),
            None => Ok(()),
        };
        if self.defers_write_errors {
            if let Err(error) = result {
                self.deferred_write_error
                    .get_or_insert_with(|| error.to_string());
            }
            Ok(())
        } else {
            result
        }
    }
}

fn allocate_runtime_log_invocation_id() -> DiagnosticLogResult<String> {
    let id = getrandom::u64().map_err(|error| {
        external_log_error("generate runtime log invocation ID", error.to_string())
    })?;
    Ok(format!("{id:016x}"))
}

impl PersistentRuntimeLogHistory {
    fn write_event(&mut self, rendered_event: &str) -> DiagnosticLogResult<()> {
        write_runtime_log_event_with_rotation(
            &self.log_dir,
            &self.path,
            &self.config,
            rendered_event,
        )?;
        Ok(())
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

fn prepare_diagnostic_log_backend(
    root: &Path,
    config: PersistentDiagnosticLogConfig,
) -> DiagnosticLogResult<PersistentRuntimeLogHistory> {
    let state_root = crate::state_paths::CanonStateRoot::resolve(root)
        .map_err(|message| external_log_error("resolve diagnostic log directory", message))?;
    prepare_diagnostic_log_backend_at(&state_root, config)
}

fn prepare_diagnostic_log_backend_at(
    state_root: &crate::state_paths::CanonStateRoot,
    config: PersistentDiagnosticLogConfig,
) -> DiagnosticLogResult<PersistentRuntimeLogHistory> {
    let log_dir = state_root.join("logs");
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
    let _lock = acquire_diagnostic_log_lock(log_dir)?;
    rotate_diagnostic_logs_with_config(log_dir, config)?;
    if active_log_size(path)?.saturating_add(line_size) > active_rotation_target {
        rotate_active_diagnostic_logs(log_dir, files)?;
    }
    prune_diagnostic_logs_to_fit(log_dir, config, line_size)?;
    // Keep file handles local to a single event. A failed write or flush then
    // returns an error without leaving poisoned writer state for the next call.
    let mut file = open_runtime_log_file(path)?;
    append_runtime_log_event_to_file(path, &mut file, line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        diagnostic_log_files, prepare_diagnostic_log_backend, DiagnosticLogConfig,
        DiagnosticLogWriter, PersistentDiagnosticLogConfig,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: hJ,hr,R1
    fn zero_limit_renders_events_without_persisting_runtime_logs() {
        let root = temp_root("diagnostic-logs-disabled");
        let mut writer =
            DiagnosticLogWriter::create_with_config(&root, DiagnosticLogConfig::Disabled).unwrap();

        writer.emit_event("info", "test.event", &[]).unwrap();
        let error = writer
            .emit_event("invalid\nlevel", "test.event", &[])
            .unwrap_err();

        assert_eq!(writer.invocation_events.len(), 1);
        assert!(writer.persistent_history.is_none());
        assert!(error.to_string().contains("not a single-line label"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: B,g2,R1
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

        assert_eq!(writer.invocation_events.len(), 1);
        assert!(writer.persistent_history.is_some());
        assert!(state_root.join("logs/0.jsonl").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 13,hr
    fn zero_limit_applies_to_existing_runtime_logs() {
        let root = git_temp_root("diagnostic-logs-disabled-zero-limit");
        let state_root = crate::state_paths::CanonStateRoot::resolve(&root).unwrap();
        let log_dir = state_root.join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(log_dir.join("0.jsonl"), "active").unwrap();
        fs::write(log_dir.join("1.jsonl"), "rotated").unwrap();

        let mut writer =
            DiagnosticLogWriter::create_with_config(&root, DiagnosticLogConfig::Disabled).unwrap();
        writer.emit_event("info", "test.event", &[]).unwrap();

        assert_eq!(log_dir_size(&log_dir), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 7N,hJ,Ky,R1
    fn deferred_write_error_is_reported_after_later_event_attempts() {
        let root = git_temp_root("diagnostic-logs-deferred-error");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let path = backend.path.clone();
        let mut writer = DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history: Some(backend),
            invocation_id: "test-invocation".to_string(),
            deferred_write_error: None,
            defers_write_errors: false,
        };
        writer.defer_write_errors();

        writer
            .emit_event("info", "check.start", &[("selected", json!(["id"]))])
            .unwrap();
        writer.emit_event("info", "second.event", &[]).unwrap();
        let error = writer.finish_deferred_writes().unwrap_err();
        let events = writer
            .invocation_events
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert!(error.contains("runtime log record is too large"));
        assert!(!path.exists());
        assert_eq!(events[0]["invocationId"], "test-invocation");
        assert_eq!(events[1]["invocationId"], events[0]["invocationId"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: m8,7N,R1
    fn persistent_diagnostic_logs_write_under_canonical_state_dir() {
        let root = git_temp_root("diagnostic-logs-canonical-state");
        let config = PersistentDiagnosticLogConfig { max_bytes: 8192 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let path = backend.path.clone();
        let mut writer = DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history: Some(backend),
            invocation_id: "test-invocation".to_string(),
            deferred_write_error: None,
            defers_write_errors: false,
        };
        writer
            .emit_event("info", "check.start", &[("selected", json!(["id"]))])
            .unwrap();
        writer.emit_event("info", "test.event", &[]).unwrap();
        assert!(path.is_file());
        let records = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["event"], "check.start");
        assert_eq!(records[1]["event"], "test.event");
        assert_eq!(records[0]["processId"], process::id());
        assert!(records
            .iter()
            .all(|record| record["processId"] == records[0]["processId"]));
        assert!(records
            .iter()
            .all(|record| record["invocationId"] == "test-invocation"));
        assert!(records
            .iter()
            .all(|record| record.get("checkStartedAt").is_none()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: fh
    fn diagnostic_logs_rotate_within_configured_log_dir_size() {
        let root = git_temp_root("diagnostic-logs-rotate-within-configured-size");
        let config = PersistentDiagnosticLogConfig { max_bytes: 4000 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let path = backend.path.clone();
        let log_dir = backend.log_dir.clone();
        let mut writer = DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history: Some(backend),
            invocation_id: "test-invocation".to_string(),
            deferred_write_error: None,
            defers_write_errors: false,
        };

        for index in 0..12 {
            writer
                .emit_event(
                    "info",
                    "test.event",
                    &[("index", json!(index)), ("payload", json!("x".repeat(120)))],
                )
                .unwrap();
        }

        assert!(path.is_file());
        assert!(log_dir.join("1.jsonl").is_file());
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: fh
    fn diagnostic_logs_make_room_for_new_event_with_rotation() {
        let root = git_temp_root("diagnostic-logs-make-room-with-rotation");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1000 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let path = backend.path.clone();
        let log_dir = backend.log_dir.clone();
        let oldest_log_file = diagnostic_log_files().last().unwrap();
        fs::write(log_dir.join(oldest_log_file), "x".repeat(950)).unwrap();
        let mut writer = DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history: Some(backend),
            invocation_id: "test-invocation".to_string(),
            deferred_write_error: None,
            defers_write_errors: false,
        };

        writer.emit_event("info", "test.event", &[]).unwrap();

        assert!(path.is_file());
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 13,m8
    fn record_larger_than_rotation_target_uses_full_directory_limit() {
        let root = git_temp_root("diagnostic-log-record-uses-directory-limit");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1024 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let log_dir = backend.log_dir.clone();
        let mut writer = DiagnosticLogWriter {
            invocation_events: Vec::new(),
            persistent_history: Some(backend),
            invocation_id: "test-invocation".to_string(),
            deferred_write_error: None,
            defers_write_errors: false,
        };

        writer
            .emit_event("info", "test.event", &[("payload", json!("x".repeat(300)))])
            .unwrap();

        let active_size = fs::metadata(log_dir.join("0.jsonl")).unwrap().len();
        assert!(active_size > super::active_log_rotation_target_bytes(&config, 8));
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: 13,fh
    fn persistent_backend_applies_size_bound_before_first_event() {
        let root = git_temp_root("diagnostic-logs-bound-on-create");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1000 };
        let backend = prepare_diagnostic_log_backend(&root, config).unwrap();
        let log_dir = backend.log_dir.clone();
        fs::write(log_dir.join("1.jsonl"), "x".repeat(700)).unwrap();
        fs::write(log_dir.join("7.jsonl"), "x".repeat(700)).unwrap();
        let unrelated_path = log_dir.join("other.log");
        fs::write(&unrelated_path, "x".repeat(600)).unwrap();

        let _writer = DiagnosticLogWriter::with_persistent_backend(backend).unwrap();

        assert!(unrelated_path.is_file());
        assert!(log_dir_size(&log_dir) <= config.max_bytes);
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

    fn log_dir_size(log_dir: &Path) -> u64 {
        fs::read_dir(log_dir)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum()
    }
}
