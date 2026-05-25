use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::Deserializer;
use std::fmt;

// The evaluator protocol intentionally makes the top-level key order part of
// the response format so logs, stdout, and human review stay predictable.
// Typed serde deserialization validates field names and types after this check;
// this visitor only records the source-order top-level keys and lets serde skip
// each value instead of maintaining a parallel JSON scanner.
pub(crate) fn validate_evaluator_response_key_order(text: &str) -> Result<(), String> {
    let keys = top_level_json_object_keys(text)?;
    if keys == ["answer", "evidence", "scope"] {
        Ok(())
    } else {
        Err(format!(
            "evaluator JSON response must contain keys in order answer, evidence, scope; got {}",
            keys.join(", ")
        ))
    }
}

pub(crate) fn top_level_json_object_keys(text: &str) -> Result<Vec<String>, String> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let keys = deserializer
        .deserialize_map(TopLevelKeyVisitor)
        .map_err(|err| format!("failed to inspect evaluator JSON object: {}", err))?;
    deserializer
        .end()
        .map_err(|_| "evaluator response must not contain surrounding prose".to_string())?;
    Ok(keys)
}

struct TopLevelKeyVisitor;

impl<'de> Visitor<'de> for TopLevelKeyVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            keys.push(key);
            let _: IgnoredAny = map.next_value()?;
        }
        Ok(keys)
    }
}
