use super::{EvaluatorConfigError, EvaluatorConfigResult};
use serde_json::Value;

pub(super) struct ConfigEntry {
    pub(super) path: Vec<String>,
    pub(super) value: ConfigEntryValue,
}

pub(super) enum ConfigEntryValue {
    String(String),
    Bool(bool),
    U64(u64),
}

impl ConfigEntry {
    pub(super) fn string<const N: usize>(path: [&str; N], value: &str) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::String(value.to_string()),
        }
    }

    pub(super) fn bool<const N: usize>(path: [&str; N], value: bool) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::Bool(value),
        }
    }

    pub(super) fn bool_path(path: Vec<String>, value: bool) -> ConfigEntry {
        ConfigEntry {
            path,
            value: ConfigEntryValue::Bool(value),
        }
    }

    pub(super) fn u64<const N: usize>(path: [&str; N], value: u64) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::U64(value),
        }
    }
}

impl ConfigEntryValue {
    fn to_json_value(&self) -> Value {
        match self {
            ConfigEntryValue::String(value) => Value::String(value.clone()),
            ConfigEntryValue::Bool(value) => Value::Bool(*value),
            ConfigEntryValue::U64(value) => Value::Number((*value).into()),
        }
    }

    pub(super) fn to_toml_value(&self) -> String {
        match self {
            ConfigEntryValue::String(value) => toml_string(value),
            ConfigEntryValue::Bool(value) => value.to_string(),
            ConfigEntryValue::U64(value) => value.to_string(),
        }
    }
}

pub(super) fn config_entries_to_json(entries: Vec<ConfigEntry>) -> EvaluatorConfigResult<Value> {
    let mut root = serde_json::Map::new();
    for entry in entries {
        insert_json_config_override(&mut root, &entry.path, entry.value.to_json_value())?;
    }
    Ok(Value::Object(root))
}

pub(super) fn insert_json_config_override(
    root: &mut serde_json::Map<String, Value>,
    path: &[String],
    value: Value,
) -> EvaluatorConfigResult<()> {
    if path.is_empty() {
        return Err(EvaluatorConfigError::Message(
            "evaluator config entry path must not be empty".to_string(),
        ));
    }
    let key = toml_dotted_key(path);
    if root.contains_key(&key) {
        return Err(EvaluatorConfigError::DuplicateConfigEntry {
            path: path.join("."),
        });
    }
    root.insert(key, value);
    Ok(())
}

pub(super) fn push_toml_arg(args: &mut Vec<String>, path: Vec<String>, value: String) {
    push_config_arg(args, &format!("{}={}", toml_dotted_key(&path), value));
}

fn toml_dotted_key(path: &[String]) -> String {
    path.iter()
        .map(|segment| toml_key_segment(segment))
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn push_config_arg(args: &mut Vec<String>, value: &str) {
    args.push("-c".to_string());
    args.push(value.to_string());
}

pub(super) fn toml_key_segment(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        value.to_string()
    } else {
        toml_string(value)
    }
}

pub(super) fn toml_string(value: &str) -> String {
    // TOML basic strings use the same delimiters and escape forms needed for
    // the values canon emits here, so the JSON string serializer gives us a
    // battle-tested quoted string. JSON may leave DEL/C1 controls literal, so
    // patch only those TOML-forbidden characters after JSON has handled the
    // common string grammar.
    let mut encoded =
        serde_json::to_string(value).expect("serializing a TOML basic string cannot fail");
    for ch in value.chars().filter(|ch| ch.is_control() && *ch > '\u{1f}') {
        encoded = encoded.replace(ch, &format!("\\u{:04X}", ch as u32));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test] // xpec: hQ
    fn config_override_paths_keep_structural_segments_bare() {
        let path = ["permissions".to_string(), "canon_check".to_string()];

        assert_eq!(toml_dotted_key(&path), "permissions.canon_check");
        assert_eq!(toml_key_segment("/tmp/**"), r#""/tmp/**""#);
    }

    #[test] // xpec: gN,hQ
    fn app_server_config_uses_flat_override_keys() {
        let config = config_entries_to_json(vec![
            ConfigEntry::string(["history", "persistence"], "none"),
            ConfigEntry::bool_path(
                vec![
                    "plugins".to_string(),
                    "plugin.with.dots".to_string(),
                    "enabled".to_string(),
                ],
                true,
            ),
        ])
        .unwrap();

        assert_eq!(
            config,
            json!({
                "history.persistence": "none",
                r#"plugins."plugin.with.dots".enabled"#: true
            })
        );
    }
}
