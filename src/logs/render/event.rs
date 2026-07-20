use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::Value;

pub(super) fn json_line(
    value: &impl Serialize,
    description: &'static str,
) -> DiagnosticLogResult<String> {
    let mut output = serde_json::to_string(value).map_err(|source| DiagnosticLogError::Json {
        description,
        source,
    })?;
    output.push('\n');
    Ok(output)
}

pub(super) struct RuntimeLogEvent<'a> {
    pub(super) timestamp: String,
    pub(super) level: &'a str,
    pub(super) event: &'a str,
    pub(super) process_id: Option<u32>,
    pub(super) invocation_id: Option<&'a str>,
    pub(super) extra: &'a [(&'a str, Value)],
}

impl Serialize for RuntimeLogEvent<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(
            3 + usize::from(self.process_id.is_some())
                + usize::from(self.invocation_id.is_some())
                + self.extra.len(),
        ))?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.serialize_entry("level", self.level)?;
        map.serialize_entry("event", self.event)?;
        if let Some(process_id) = self.process_id {
            map.serialize_entry("processId", &process_id)?;
        }
        if let Some(invocation_id) = self.invocation_id {
            map.serialize_entry("invocationId", invocation_id)?;
        }
        for (key, value) in self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
