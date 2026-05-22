use crate::config_types::AgentConfig;
use crate::scope::{is_denied_path, normalize_repo_path, sanitize_scope};

#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
pub(crate) fn parse_scope_json(text: &str, agent: &AgentConfig) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("failed to parse SCOPE JSON: {}", err))?;
    let array = value
        .as_array()
        .ok_or("SCOPE must be a JSON array".to_string())?;
    let mut scope = Vec::new();
    for item in array {
        let raw = item
            .as_str()
            .ok_or("SCOPE entries must be strings".to_string())?;
        scope.push(raw.to_string());
    }
    parse_scope_strings(&scope, agent)
}

pub(crate) fn parse_scope_strings(
    scope: &[String],
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    // Scope parsing is deliberately limited to representation and access
    // checks. Whether a narrowed scope is sufficient for the answer is tested
    // later by running an independent interrogation under that proposed scope.
    let mut parsed = Vec::new();
    for raw in scope {
        let normalized = normalize_repo_path(raw)?;
        if normalized != *raw {
            return Err(format!("scope entry must be normalized: {}", raw));
        }
        if normalized != "." && is_denied_path(agent, &normalized) {
            return Err(format!("scope entry is denied: {}", raw));
        }
        parsed.push(normalized);
    }
    sanitize_scope(&parsed, agent)
}
