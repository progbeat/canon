use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use crate::logs::DiagnosticLogWriter;
use serde_json::{json, Value};

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    event: &ThreadLifecycleEventFields<'_>,
) -> DiagnosticLogResult<()> {
    if !matches!(event.event, "thread.start" | "thread.reuse") {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: "event".to_string(),
            reason: "thread lifecycle helper accepts only thread.start or thread.reuse",
        });
    }
    diagnostic_log.emit_event(
        "info",
        event.event,
        &[
            ("threadId", json!(event.thread_id)),
            ("id", json!(event.expectation_id)),
            ("scope", json!(event.scope)),
            ("model", json!(event.model)),
            ("thinking", json!(event.thinking)),
            ("baseInstructions", json!(event.base_instructions)),
            ("developerInstructions", json!(event.developer_instructions)),
            ("evaluationContext", json!(event.evaluation_context)),
        ],
    )
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    event: &ThreadRestartEventFields<'_>,
) -> DiagnosticLogResult<()> {
    let fields = thread_restart_event_fields(event);
    diagnostic_log.emit_event("warn", "thread.restart", &fields)
}

fn thread_restart_event_fields(event: &ThreadRestartEventFields<'_>) -> [(&'static str, Value); 7] {
    // [kK] Restart is a distinct event from the following fresh-thread start.
    // Its effective instruction pair is assembled exactly once here.
    [
        ("threadId", json!(event.thread_id)),
        ("id", json!(event.expectation_id)),
        ("scope", json!(event.scope)),
        ("model", json!(event.model)),
        ("baseInstructions", json!(event.base_instructions)),
        ("developerInstructions", json!(event.developer_instructions)),
        ("reason", json!(event.reason)),
    ]
}

pub(crate) struct ThreadLifecycleEventFields<'a> {
    pub(crate) event: &'a str,
    pub(crate) thread_id: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) base_instructions: &'a str,
    pub(crate) developer_instructions: &'a str,
    pub(crate) evaluation_context: &'a crate::evaluator::ThreadEvaluationLogContext,
}

pub(crate) struct ThreadRestartEventFields<'a> {
    pub(crate) thread_id: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) base_instructions: &'a str,
    pub(crate) developer_instructions: &'a str,
    pub(crate) reason: &'a str,
}
