use crate::check::types::CheckRecord;
use crate::fs_util::ensure_dir_without_symlinks;
use crate::logs::config::{
    active_log_file_name, diagnostic_log_files, diagnostic_logs_explicitly_disabled,
};
use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::logs::lock::acquire_diagnostic_log_lock;
use crate::logs::rotation::{
    active_log_size, append_runtime_log_event_to_file, open_runtime_log_file,
    prune_diagnostic_logs_to_limit, rotate_active_diagnostic_logs,
    rotate_diagnostic_logs_with_config,
};
use crate::repo_inspection::RepoInspectionCache;
use crate::{DiagnosticLogConfig, CANON_LOG_DIR_GIT_PATH};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) use crate::history::render_answer_history_record;
pub(crate) use crate::logs::config::diagnostic_log_config;
pub(crate) use crate::logs::error::DiagnosticLogError;
#[cfg(test)]
pub(crate) use crate::logs::lock::{
    stale_diagnostic_log_lock_age, write_diagnostic_log_lock_token,
};
pub(crate) use crate::logs::render::{push_json_control_escape, render_runtime_log_event};

#[cfg(test)]
pub(crate) fn write_diagnostic_log(
    root: &Path,
    records: &[CheckRecord],
) -> DiagnosticLogResult<PathBuf> {
    let mut writer = DiagnosticLogWriter::create(root)?;
    for record in records {
        writer.write_record(record)?;
    }
    let path = writer.path.clone();
    Ok(path)
}

pub(crate) struct DiagnosticLogWriter {
    path: PathBuf,
    log_dir: PathBuf,
    config: DiagnosticLogConfig,
}

impl DiagnosticLogWriter {
    // This module owns JSONL storage and rotation. `logging_render` validates
    // the required fields for known runtime-log event schemas.
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

    pub(crate) fn write_record(&mut self, record: &CheckRecord) -> DiagnosticLogResult<()> {
        self.write_record_event("expectation.result", record)
    }

    pub(crate) fn write_interrogation_record(
        &mut self,
        record: &CheckRecord,
    ) -> DiagnosticLogResult<()> {
        self.write_record_event("interrogation.result", record)
    }

    fn write_record_event(&mut self, event: &str, record: &CheckRecord) -> DiagnosticLogResult<()> {
        let fields = record_log_fields(record);
        self.write_event("info", event, &fields)?;
        if record_requires_human_review(record) {
            let review_event = match event {
                "expectation.result" => "expectation.review_required",
                "interrogation.result" => "interrogation.review_required",
                _ => return Ok(()),
            };
            self.write_event("warn", review_event, &fields)?;
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
        ("result", json!(record.result)),
        ("observed", json!(record.observed)),
        ("evidence", json!(record.evidence)),
        ("scope", json!(record.scope)),
        ("prompt", json!(record.prompt_text())),
        ("expected", json!(record.expected_text())),
    ]
}

fn record_requires_human_review(record: &CheckRecord) -> bool {
    record.review_error_text().is_some()
}

#[cfg(test)]
pub(crate) fn append_runtime_log_event(
    root: &Path,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    let mut cache = RepoInspectionCache::new();
    let prepared = prepare_diagnostic_log(root, &mut cache)?;
    write_runtime_log_event_with_rotation(
        &prepared.log_dir,
        &prepared.path,
        &prepared.config,
        level,
        event,
        fields,
    )
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
    // CANON_LOG_DIR_GIT_PATH is `${CANON_STATE_DIR}/logs`; `git_path` resolves it
    // with `git rev-parse --git-path` so worktrees and nonstandard git-dir
    // layouts keep logs under Canon's git-owned state directory.
    let log_dir = cache
        .git_path(root, CANON_LOG_DIR_GIT_PATH)
        .map_err(|message| external_log_error("resolve diagnostic log directory", message))?;
    if !diagnostic_logs_explicitly_disabled(&config) {
        ensure_dir_without_symlinks(&log_dir)
            .map_err(|message| external_log_error("create diagnostic log directory", message))?;
    }
    let path = log_dir.join(active_log_file_name(&config)?);
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
    if log_size_limited && line_size > config.max_bytes {
        return Err(DiagnosticLogError::RecordTooLarge {
            size: line_size,
            max_bytes: config.max_bytes,
        });
    }
    let _lock = acquire_diagnostic_log_lock(log_dir)?;
    rotate_diagnostic_logs_with_config(log_dir, config)?;
    if log_size_limited && active_log_size(path)?.saturating_add(line_size) > config.max_bytes {
        rotate_active_diagnostic_logs(log_dir, diagnostic_log_files(config)?)?;
    }
    // Keep file handles local to a single event. A failed write or flush then
    // returns an error without leaving poisoned writer state for the next call.
    let mut file = open_runtime_log_file(path)?;
    append_runtime_log_event_to_file(path, &mut file, &line)?;
    drop(file);
    prune_diagnostic_logs_to_limit(log_dir, config)
}
