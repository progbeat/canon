use crate::logs::error::DiagnosticLogResult;
use crate::logs::DiagnosticLogWriter;
use serde_json::json;

pub(crate) fn write_xpec_state_retention_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    removed: usize,
    kept: usize,
) -> DiagnosticLogResult<()> {
    diagnostic_log.emit_event(
        "info",
        "xpec_state.retention",
        &[("removed", json!(removed)), ("kept", json!(kept))],
    )
}
