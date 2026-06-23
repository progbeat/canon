use crate::check::command::output::{
    start_expectation_report_output, SharedCheckOutput, StartedExpectationReportOutput,
};
use crate::check::core::{CheckRecord, SelectedExpectation};
use crate::evaluator::EvaluatorProgress;
use crate::fs_util::{ensure_dir_without_symlinks, reject_symlink};
use crate::git::resolve_git_path;
use crate::state_paths::CANON_LIVE_REPORT_DIR_GIT_PATH;
use crate::time::{format_record_timestamp, unix_timestamp};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) enum LiveExpectationReport {
    StateBacked(StateBackedLiveExpectationReport),
    OutputOnly(StartedExpectationReportOutput),
}

// State-backed live expectation reports have only start and finish operations.
// The state file is written before the public short-ID prefix, so even a later
// interruption or public-output failure has already reported the expectation.
// There is intentionally no cancel operation.
pub(super) struct StateBackedLiveExpectationReport {
    output: StartedExpectationReportOutput,
    state_path: PathBuf,
}

pub(super) fn start_live_expectation_report(
    state_root: Option<&Path>,
    output: &SharedCheckOutput,
    expectation: &SelectedExpectation,
) -> Result<LiveExpectationReport, String> {
    let Some(root) = state_root else {
        return Ok(LiveExpectationReport::OutputOnly(
            start_expectation_report_output(output.clone(), &expectation.display_id),
        ));
    };
    let state_path = live_report_state_path(root, expectation)?;
    // Write and flush a canon-owned report marker before the public short-ID
    // prefix. If public output later fails or the process is interrupted after
    // the prefix, this state file is still a report for the expectation.
    write_live_report_state(
        &state_path,
        json!({
            "timestamp": format_record_timestamp(unix_timestamp()?),
            "status": "started",
            "id": expectation.id,
            "displayId": expectation.display_id,
            "question": expectation.question,
        }),
    )?;
    Ok(LiveExpectationReport::StateBacked(
        StateBackedLiveExpectationReport {
            output: start_expectation_report_output(output.clone(), &expectation.display_id),
            state_path,
        },
    ))
}

impl LiveExpectationReport {
    pub(super) fn progress(&self) -> EvaluatorProgress {
        match self {
            LiveExpectationReport::StateBacked(report) => report.output.progress(),
            LiveExpectationReport::OutputOnly(output) => output.progress(),
        }
    }

    pub(super) fn finish_public_output_or_keep_state_report(self, record: &CheckRecord) {
        match self {
            LiveExpectationReport::StateBacked(report) => {
                report.finish_public_output_or_keep_state_report(record)
            }
            LiveExpectationReport::OutputOnly(output) => {
                let _ = output.finish_with_record(record);
            }
        }
    }
}

impl StateBackedLiveExpectationReport {
    fn finish_public_output_or_keep_state_report(self, record: &CheckRecord) {
        let _ = write_live_report_state(
            &self.state_path,
            json!({
                "timestamp": record.timestamp,
                "status": "completed",
                "id": record.id,
                "displayId": record.display_id,
                "result": record.result,
                "question": record.question_text(),
                "expected": record.expected_answer_text(),
                "observed": record.observed,
                "error": record.error,
                "evidence": record.evidence,
                "visibleScope": record.scope,
                "visibleTreeOid": record.visible_tree_oid,
            }),
        );
        if self.output.finish_with_record(record) {
            // Public output already contains the report; keep the state file
            // only for interrupted or public-output-failure cases.
            let _ = fs::remove_file(&self.state_path);
        } else {
            // Both public sinks refused the completion. The completed state
            // report above remains under CANON_STATE_DIR/live-reports.
        }
    }
}

fn live_report_state_path(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<PathBuf, String> {
    let dir = resolve_git_path(root, CANON_LIVE_REPORT_DIR_GIT_PATH)?;
    Ok(dir.join(format!("{}.json", expectation.id)))
}

fn write_live_report_state(path: &Path, value: Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(path)?;
    // Live reports are single-record snapshots. Truncating replaces only this
    // one report record, so write work is linear in the newly persisted JSON
    // bytes rather than accumulated retained state.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    let line = value.to_string();
    file.write_all(line.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}
