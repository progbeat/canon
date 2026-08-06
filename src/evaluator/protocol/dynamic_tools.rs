use super::super::tool_schema::load_dynamic_tools;
use serde_json::Value;
use std::sync::OnceLock;

// [UZ,6,d] The evaluator protocol owns its agent-facing tool schema resource,
// parses it once, and exposes only the typed protocol value to callers.
const CANON_SHOW_DYNAMIC_TOOLS_RESOURCE: &str =
    include_str!("../../../resources/prompts/canon_show_dynamic_tools.json");
// [fh] This cache is process-local. Loading or cloning the schema performs no
// persistent write and never accesses CANON_STATE_DIR.
static CANON_SHOW_DYNAMIC_TOOLS: OnceLock<Result<Vec<Value>, String>> = OnceLock::new();

pub(crate) fn canon_show_dynamic_tools() -> Result<Vec<Value>, String> {
    load_dynamic_tools(
        &CANON_SHOW_DYNAMIC_TOOLS,
        CANON_SHOW_DYNAMIC_TOOLS_RESOURCE,
        "canon.show",
    )
}

#[cfg(test)]
mod tests {
    use super::canon_show_dynamic_tools;

    #[test] // xpec: UZ,6
    fn canon_show_dynamic_tools_load_from_resource() {
        let dynamic_tools = canon_show_dynamic_tools().unwrap();

        assert_eq!(dynamic_tools.len(), 1);
        assert_eq!(dynamic_tools[0]["name"], "canon");
        assert_eq!(dynamic_tools[0]["tools"][0]["name"], "show");
    }
}
