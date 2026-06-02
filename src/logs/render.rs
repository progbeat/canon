use crate::logs::error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
use crate::time::{format_record_timestamp, unix_timestamp};
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn render_runtime_log_event(
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<String> {
    validate_runtime_log_extra_fields(fields)?;
    validate_runtime_log_event_schema(event, fields)?;
    let event = RuntimeLogEvent {
        timestamp: format_record_timestamp(
            unix_timestamp().map_err(|message| external_log_error("read system time", message))?,
        ),
        level,
        event,
        extra: fields,
    };
    json_line(&event, "runtime log event")
}

fn validate_runtime_log_event_schema(
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    let Some(required) = required_runtime_log_fields(event) else {
        return Ok(());
    };
    for key in required {
        if runtime_log_field_value(fields, key).is_some() {
            continue;
        }
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: (*key).to_string(),
            reason: "missing for event schema",
        });
    }
    validate_runtime_log_nested_schema(event, fields)?;
    Ok(())
}

fn required_runtime_log_fields(event: &str) -> Option<&'static [&'static str]> {
    match event {
        "agent.request" => Some(&["id", "attempt", "reason", "request"]),
        "agent.response" => Some(&["id", "attempt", "reason", "response"]),
        "agent.turn_error" => Some(&["id", "attempt", "reason", "error", "response"]),
        "cache.cleanup" => Some(&["removed", "kept"]),
        "cache.hit" => Some(&["id", "result", "scope"]),
        "check.start" => Some(&["selected"]),
        "expectation.result" | "interrogation.result" => Some(&[
            "id", "result", "observed", "evidence", "scope", "prompt", "expected",
        ]),
        "expectation.review_required" | "interrogation.review_required" => Some(&[
            "id", "result", "observed", "evidence", "scope", "prompt", "expected", "reason",
        ]),
        "lazy_full_scope_reset" => Some(&["evaluated", "candidates", "reset", "ids"]),
        "lazy_full_scope_reset.error" => Some(&["message"]),
        "model.failure" => Some(&["id", "model", "error"]),
        "model.fallback" => Some(&["id", "from", "to", "reason"]),
        "query.result" => Some(&["prompt", "observed", "evidence", "suggestedQScope"]),
        "query.review_required" => Some(&[
            "prompt",
            "observed",
            "evidence",
            "suggestedQScope",
            "reason",
        ]),
        "scope.narrowing" => Some(&[
            "id",
            "originalScope",
            "proposedScope",
            "accepted",
            "initialObserved",
            "initialEvidence",
            "verificationObserved",
            "verificationEvidence",
        ]),
        "thread.restart" => Some(&[
            "threadId",
            "id",
            "scope",
            "model",
            "baseInstructions",
            "developerInstructions",
            "reason",
        ]),
        "thread.start" | "thread.reuse" => Some(&[
            "threadId",
            "scope",
            "model",
            "thinking",
            "baseInstructions",
            "developerInstructions",
        ]),
        "check.finish" => Some(&["query", "status"]),
        _ => None,
    }
}

fn validate_runtime_log_nested_schema(
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
    if !has_token_usage && !has_token_usage_updates {
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: "tokenUsage".to_string(),
            reason: "missing usage source for event schema",
        });
    }
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

fn runtime_log_field_value<'a>(fields: &'a [(&str, Value)], key: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find_map(|(field, value)| (*field == key).then_some(value))
}

fn validate_runtime_log_extra_fields(fields: &[(&str, Value)]) -> DiagnosticLogResult<()> {
    let mut seen = BTreeSet::new();
    for (key, _) in fields {
        if matches!(*key, "timestamp" | "level" | "event") {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key: (*key).to_string(),
                reason: "reserved",
            });
        }
        if !seen.insert(*key) {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key: (*key).to_string(),
                reason: "duplicated",
            });
        }
    }
    Ok(())
}

fn json_line(value: &impl Serialize, description: &'static str) -> DiagnosticLogResult<String> {
    let mut output = serde_json::to_string(value).map_err(|source| DiagnosticLogError::Json {
        description,
        source,
    })?;
    output.push('\n');
    Ok(output)
}

struct RuntimeLogEvent<'a> {
    timestamp: String,
    level: &'a str,
    event: &'a str,
    extra: &'a [(&'a str, Value)],
}

impl Serialize for RuntimeLogEvent<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(3 + self.extra.len()))?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.serialize_entry("level", self.level)?;
        map.serialize_entry("event", self.event)?;
        for (key, value) in self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

pub(crate) fn push_json_control_escape(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let code = byte as usize;
    output.push_str("\\u00");
    output.push(HEX[(code >> 4) & 0x0f] as char);
    output.push(HEX[code & 0x0f] as char);
}

#[cfg(test)]
mod tests {
    use super::render_runtime_log_event;
    use serde_json::{json, Value};

    #[test]
    fn token_usage_updates_expose_raw_count_objects() {
        let fields = agent_response_fields(vec![json!({
            "sequence": 1,
            "threadId": "thread",
            "turnId": "turn",
            "tokenUsage": {
                "last": token_usage(),
                "total": token_usage(),
            },
        })]);

        render_runtime_log_event("info", "agent.response", &fields).unwrap();
    }

    #[test]
    fn token_usage_updates_require_reasoning_count() {
        let mut usage = token_usage();
        usage
            .as_object_mut()
            .unwrap()
            .remove("reasoningOutputTokens");
        let fields = agent_response_fields(vec![json!({
            "sequence": 1,
            "threadId": "thread",
            "turnId": "turn",
            "tokenUsage": {
                "last": usage,
                "total": token_usage(),
            },
        })]);

        let error = render_runtime_log_event("info", "agent.response", &fields).unwrap_err();

        assert!(error.to_string().contains("reasoningOutputTokens"));
    }

    #[test]
    fn review_required_record_events_include_reason() {
        let fields = vec![
            ("id", json!("id")),
            ("result", json!("fail")),
            ("observed", json!("unparsable")),
            ("evidence", json!("evidence")),
            ("scope", json!(["."])),
            ("prompt", json!("prompt")),
            ("expected", json!("yes")),
            ("reason", json!("unparsable")),
        ];

        render_runtime_log_event("warn", "expectation.review_required", &fields).unwrap();
    }

    fn agent_response_fields(updates: Vec<Value>) -> Vec<(&'static str, Value)> {
        vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("response", json!({"sessionId": "thread", "text": "{}"})),
            ("tokenUsageUpdates", json!(updates)),
        ]
    }

    fn token_usage() -> Value {
        json!({
            "totalTokens": 10,
            "inputTokens": 6,
            "cachedInputTokens": 2,
            "outputTokens": 4,
            "reasoningOutputTokens": 1,
        })
    }
}
