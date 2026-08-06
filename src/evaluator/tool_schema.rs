use serde_json::Value;
use std::sync::OnceLock;

pub(super) fn load_dynamic_tools(
    cache: &OnceLock<Result<Vec<Value>, String>>,
    resource: &str,
    label: &str,
) -> Result<Vec<Value>, String> {
    cache
        .get_or_init(|| {
            serde_json::from_str(resource)
                .map_err(|err| format!("failed to parse {label} dynamic tools: {err}"))
        })
        .clone()
}
