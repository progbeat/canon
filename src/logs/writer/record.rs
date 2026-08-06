use super::DiagnosticLogWriter;
use crate::check::CheckRecord;
use crate::logs::error::DiagnosticLogResult;
use serde_json::{json, Value};

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
    // [w] One completed record produces its parsed result event and, when its
    // normalized error requires human review, the accompanying diagnostic.
    // Callers therefore cannot accidentally log the outcome without the
    // review-required event.
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
