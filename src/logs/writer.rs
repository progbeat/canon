mod backend;
mod record;

use crate::logs::config::{DiagnosticLogConfig, DiagnosticLogPlan};
use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::logs::render::render_runtime_log_process_event;
use backend::{
    disable_persistent_storage_at, prepare_diagnostic_log_backend, PersistentRuntimeLogHistory,
};
use serde_json::Value;
use std::path::Path;

pub(crate) use record::DiagnosticRecordEvent;

/// Unconditional command-facing runtime-event entry point.
///
/// Commands construct every applicable event through this API. The writer
/// renders and validates every event and retains it in invocation memory.
/// Writers created for persistent commands also append to bounded
/// cross-invocation JSONL history under `CANON_STATE_DIR` when configured;
/// temporary-query writers deliberately remain memory-only.
/// No command call site branches on the selected storage.
pub(crate) struct DiagnosticLogWriter {
    invocation_events: Vec<String>,
    persistent_history: Option<PersistentRuntimeLogHistory>,
    invocation_id: String,
    deferred_write_error: Option<String>,
    defers_write_errors: bool,
}

impl DiagnosticLogWriter {
    // Runtime-event ownership is intentionally centralized here. Every
    // `emit_event` call renders and validates one complete event and retains
    // its JSONL representation in invocation memory. A writer constructed with
    // persistent history separately attempts to append it under
    // `${CANON_STATE_DIR}/logs`.
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
        persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        // [90] This is not the absent-value branch of Git-backed diagnostic
        // configuration. In-place is a separate command mode that must ignore
        // all Git information even when `canon.logs.maxSize` is configured.
        // Its runtime events therefore remain in invocation memory without
        // constructing a `DiagnosticLogConfig` at all.
        Self::without_persistent_storage_at(persistent_state_root)
    }

    pub(crate) fn create_temporary_query(
        git_backed_plan: Option<DiagnosticLogPlan>,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        // [l,g2,kK] Ask validates the command's log configuration and produces
        // the same runtime events, but its one-off query retains those events
        // only in invocation memory. It therefore cannot reach the persistent
        // backend or its cross-process coordination path.
        if let Some(plan) = git_backed_plan {
            let _validated_config = plan.into_config()?;
        }
        Self::without_persistent_storage()
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
        disable_persistent_storage_at(state_root)?;
        Self::without_persistent_storage()
    }

    fn with_persistent_backend(
        backend: PersistentRuntimeLogHistory,
    ) -> DiagnosticLogResult<DiagnosticLogWriter> {
        backend.activate()?;
        Self::with_optional_persistent_history(Some(backend))
    }

    /// Keeps runtime observability failures from interrupting the operation
    /// whose events are being recorded. Event writes are still attempted at
    /// every call site; the first failure is returned by
    /// `finish_deferred_writes` after the operation's required effects.
    pub(crate) fn defer_write_errors(&mut self) {
        debug_assert!(!self.defers_write_errors); // xpec: kK,l,Yq
        debug_assert!(self.deferred_write_error.is_none()); // xpec: kK,l,Yq
        self.defers_write_errors = true;
    }

    pub(crate) fn finish_deferred_writes(&mut self) -> Result<(), String> {
        debug_assert!(self.defers_write_errors); // xpec: kK,l,Yq
        self.defers_write_errors = false;
        match self.deferred_write_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn emit_event(
        &mut self,
        level: &str,
        event: &str,
        fields: &[(&str, Value)],
    ) -> DiagnosticLogResult<()> {
        // [kK,hr] Rendering and validating the complete runtime event is
        // unconditional. Storage policy is an internal concern and controls
        // only whether that event receives a persistent JSONL representation.
        let rendered = render_runtime_log_process_event(&self.invocation_id, level, event, fields)?;
        // [g2,kK,Yq] Every valid event and its primary invocation correlation
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

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::logs::config::PersistentDiagnosticLogConfig;
    use serde_json::{json, Value};
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};

    #[test] // xpec: kK,l,Yq
    fn deferred_write_error_is_reported_after_later_event_attempts() {
        let root = git_temp_root("diagnostic-logs-deferred-error");
        let config = PersistentDiagnosticLogConfig { max_bytes: 1 };
        let mut writer =
            DiagnosticLogWriter::create_with_config(&root, DiagnosticLogConfig::Persistent(config))
                .unwrap();
        writer.defer_write_errors();

        writer
            .emit_event("info", "check.start", &[("candidates", json!(["id"]))])
            .unwrap();
        writer.emit_event("info", "second.event", &[]).unwrap();
        let error = writer.finish_deferred_writes().unwrap_err();
        let events = writer
            .invocation_events
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert!(error.contains("runtime log record is too large"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["invocationId"], events[0]["invocationId"]);
        fs::remove_dir_all(root).unwrap();
    }

    fn git_temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canon-test-{name}-{}-{:016x}",
            process::id(),
            getrandom::u64().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .unwrap();
        // xpec: kK,l,Yq
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }
}
