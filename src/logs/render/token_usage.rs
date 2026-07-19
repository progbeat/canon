use super::schema::runtime_log_field_value;
use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use serde_json::Value;

pub(super) fn validate_agent_token_usage_schema(
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    if event != "agent.token_usage" {
        return Ok(());
    }
    let Some(token_usage) = runtime_log_field_value(fields, "tokenUsage") else {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: "tokenUsage".to_string(),
            reason: "missing for event schema",
        });
    };
    validate_token_usage_counts("tokenUsage", token_usage)?;
    Ok(())
}

fn validate_token_usage_counts(prefix: &str, value: &Value) -> DiagnosticLogResult<()> {
    let Some(object) = value.as_object() else {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: prefix.to_string(),
            reason: "not an object",
        });
    };
    for key in [
        "totalTokens",
        "inputTokens",
        "cachedInputTokens",
        "outputTokens",
        "reasoningOutputTokens",
    ] {
        let Some(value) = object.get(key) else {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key: format!("{}.{}", prefix, key),
                reason: "missing for event schema",
            });
        };
        if value.as_u64().is_some() {
            continue;
        }
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: format!("{}.{}", prefix, key),
            reason: "not an unsigned integer",
        });
    }
    Ok(())
}
