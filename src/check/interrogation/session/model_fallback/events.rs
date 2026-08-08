use crate::logs::DiagnosticLogWriter;
use serde_json::json;

pub(crate) fn write_model_fallback_events(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    model: Option<&str>,
    next_model: Option<&str>,
    error: &str,
) -> crate::logs::DiagnosticLogResult<()> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer.emit_event(
        "warn",
        "model.failure",
        &[
            ("id", json!(expectation_id)),
            ("model", json!(model)),
            ("error", json!(error)),
        ],
    )?;
    if let Some(next_model) = next_model {
        writer.emit_event(
            "warn",
            "model.fallback",
            &[
                ("id", json!(expectation_id)),
                ("from", json!(model)),
                ("to", json!(next_model)),
                ("reason", json!(error)),
            ],
        )?;
    }
    Ok(())
}
