use serde_json::{json, Value};

pub(crate) fn scope_narrowing_log_fields(
    id: &str,
    original_scope: &[String],
    proposed_scope: &[String],
    accepted: bool,
) -> Vec<(&'static str, Value)> {
    vec![
        ("id", json!(id)),
        ("originalScope", json!(original_scope)),
        ("proposedScope", json!(proposed_scope)),
        ("accepted", json!(accepted)),
    ]
}
