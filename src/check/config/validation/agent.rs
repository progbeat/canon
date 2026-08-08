use crate::check::core::contains_line_break;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::scope::normalize_repo_path;

pub(crate) fn validate_resolved_agent_config(
    agent: &AgentConfig,
    label: &str,
) -> Result<(), String> {
    for (index, model) in agent.models.iter().enumerate() {
        validate_optional_model(
            Some(model.as_str()),
            &format!("{}.models[{}]", label, index),
        )?;
    }
    validate_thinking(&agent.thinking).map_err(|err| format!("{}: {}", label, err))?;
    for plugin in &agent.plugins {
        validate_plugin_config_key(plugin)?;
    }
    Ok(())
}

pub(crate) fn validate_plugin_config_key(value: &str) -> Result<(), String> {
    // Plugin keys are forwarded verbatim to the app server. Reject whitespace
    // instead of trimming so the runtime key matches the visible config token.
    if value.trim().is_empty() {
        return Err("agent has an empty plugin entry".to_string());
    }
    if value != value.trim() {
        return Err("agent plugin entries must not have surrounding whitespace".to_string());
    }
    if contains_line_break(value) {
        return Err("agent plugin entries must be single-line strings".to_string());
    }
    if value.chars().any(char::is_whitespace) {
        return Err("agent plugin entries must not contain whitespace".to_string());
    }
    let Some((plugin, marketplace)) = value.split_once('@') else {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    };
    if plugin.is_empty() || marketplace.is_empty() || marketplace.contains('@') {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    }
    if !is_plugin_key_segment(plugin) || !is_plugin_key_segment(marketplace) {
        return Err(format!(
            "agent plugin entry segments must be lowercase kebab-case: {}",
            value
        ));
    }
    Ok(())
}

fn is_plugin_key_segment(value: &str) -> bool {
    if value.is_empty() || value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(crate) fn validate_optional_model(value: Option<&str>, label: &str) -> Result<(), String> {
    let Some(model) = value else {
        return Ok(());
    };
    // Model IDs are forwarded verbatim to the app server. This syntax-only
    // validation rejects invisible or whitespace variants of otherwise valid
    // IDs, while leaving the live model/capability matrix to the app server.
    if model.trim().is_empty() {
        return Err(format!("check.yml {} must not be empty", label));
    }
    if model != model.trim() {
        return Err(format!(
            "check.yml {} must not have surrounding whitespace",
            label
        ));
    }
    if model.chars().any(char::is_control) {
        return Err(format!(
            "check.yml {} must not contain control characters",
            label
        ));
    }
    if !model.is_ascii() {
        return Err(format!("check.yml {} must be ASCII", label));
    }
    if model.chars().any(char::is_whitespace) {
        return Err(format!("check.yml {} must not contain whitespace", label));
    }
    Ok(())
}

pub(crate) fn validate_thinking(value: &str) -> Result<(), String> {
    // Thinking validation is independent of the selected model for the same
    // reason as model-name validation: capability checks belong at the
    // app-server boundary, not in static config parsing.
    if value.trim().is_empty() {
        return Err("thinking must not be empty".to_string());
    }
    if contains_line_break(value) {
        return Err("thinking must be a single-line string".to_string());
    }
    match value {
        "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => Ok(()),
        _ => Err(format!("unsupported thinking: {}", value)),
    }
}

pub(crate) fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
        || config
            .expectations
            .iter()
            .any(|expectation| !expectation.agent.plugins.is_empty())
}

pub(crate) fn normalize_agent_ignore_pattern_for_config(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("agent ignore pattern: path must not be empty".to_string());
    }
    normalize_repo_path(value).map_err(|err| format!("agent ignore pattern: {}", err))
}
