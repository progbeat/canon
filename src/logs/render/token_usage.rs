use super::schema::runtime_log_field_value;
use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use serde_json::Value;

pub(super) fn validate_agent_token_usage_schema(
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    if !matches!(event, "agent.response" | "agent.turn_error") {
        return Ok(());
    }
    let token_usage_updates = runtime_log_field_value(fields, "tokenUsageUpdates");
    let has_token_usage_updates = match token_usage_updates {
        Some(value) => {
            let Some(updates) = value.as_array() else {
                return Err(DiagnosticLogError::InvalidRuntimeField {
                    key: "tokenUsageUpdates".to_string(),
                    reason: "not an array",
                });
            };
            if updates.is_empty() {
                return Err(DiagnosticLogError::InvalidRuntimeField {
                    key: "tokenUsageUpdates".to_string(),
                    reason: "empty for event schema",
                });
            }
            for (index, update) in updates.iter().enumerate() {
                validate_token_usage_update(index, update)?;
            }
            true
        }
        None => false,
    };
    let has_token_usage = runtime_log_field_value(fields, "tokenUsage").is_some();
    if has_token_usage && has_token_usage_updates {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: "tokenUsage".to_string(),
            reason: "duplicates raw token usage updates",
        });
    }
    if let Some(token_usage) = runtime_log_field_value(fields, "tokenUsage") {
        validate_token_usage_counts("tokenUsage", token_usage)?;
    }
    Ok(())
}

fn validate_token_usage_update(index: usize, update: &Value) -> DiagnosticLogResult<()> {
    let prefix = format!("tokenUsageUpdates[{}]", index);
    let Some(object) = update.as_object() else {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: prefix,
            reason: "not an object",
        });
    };
    for key in ["sequence", "threadId", "turnId", "tokenUsage"] {
        if object.get(key).is_some() {
            continue;
        }
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: format!("{}.{}", prefix, key),
            reason: "missing for event schema",
        });
    }
    let token_usage_key = format!("{}.tokenUsage", prefix);
    let token_usage = object.get("tokenUsage").expect("presence checked above");
    let Some(token_usage) = token_usage.as_object() else {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: token_usage_key,
            reason: "not an object",
        });
    };
    for part in ["last", "total"] {
        let key = format!("{}.tokenUsage.{}", prefix, part);
        let Some(value) = token_usage.get(part) else {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key,
                reason: "missing for event schema",
            });
        };
        validate_token_usage_counts(&key, value)?;
    }
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
