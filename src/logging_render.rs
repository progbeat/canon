use crate::check_types::{CheckRecord, CheckResult};
use crate::logging_error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
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

pub(crate) fn render_check_log_record(record: &CheckRecord) -> DiagnosticLogResult<String> {
    // History records intentionally start with the cache.md required field
    // prefix. Extra persisted metadata follows it; expectation references use
    // the resolved full ID, never the display/selector prefix.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        result: record.result,
        observed: &record.observed,
        evidence: &record.evidence,
        scope: &record.scope,
        scope_tree_oid: &record.scope_hash,
        id: &record.id,
        prompt: record.prompt_text(),
        expected: record.expected_text(),
        cache_key: record.cache_key.as_deref(),
    };
    json_line(&history, "history log record")
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

#[derive(Serialize)]
struct HistoryLogRecord<'a> {
    timestamp: &'a str,
    result: CheckResult,
    observed: &'a str,
    evidence: &'a str,
    scope: &'a [String],
    #[serde(rename = "scopeTreeOid")]
    scope_tree_oid: &'a str,
    id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<&'a str>,
    #[serde(rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    cache_key: Option<&'a str>,
}

pub(crate) fn push_json_control_escape(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let code = byte as usize;
    output.push_str("\\u00");
    output.push(HEX[(code >> 4) & 0x0f] as char);
    output.push(HEX[code & 0x0f] as char);
}
