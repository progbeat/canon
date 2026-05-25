use crate::check_selection::parse_cooldown;
use crate::check_types::contains_line_break;
use crate::config_types::CheckConfig;
use crate::scope::normalize_repo_path;
use crate::{OBSERVED_IDK, OBSERVED_MALFORMED};

pub(crate) fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    // Prompt rendering trims optional custom instructions; reject explicitly
    // blank-looking values so a configured policy cannot silently disappear.
    if let Some(instructions) = config.agent.instructions.as_deref() {
        if !contains_visible_config_text(instructions) {
            return Err("check.yml agent.instructions must contain visible text".to_string());
        }
    }
    validate_optional_model(config.agent.model.primary.as_deref(), "agent.model.primary")?;
    for (index, model) in config.agent.model.fallbacks.iter().enumerate() {
        validate_optional_model(
            Some(model.as_str()),
            &format!("agent.model.fallbacks[{}]", index),
        )?;
    }
    validate_thinking(&config.agent.thinking)?;
    for path in &config.agent.ignore {
        normalize_agent_ignore_pattern_for_config(path)?;
    }
    for plugin in &config.agent.plugins {
        validate_plugin_config_key(plugin)?;
    }
    if config.expectations.is_empty() {
        return Err("check.yml expectations must not be empty".to_string());
    }
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        if !contains_visible_config_text(&expectation.q) {
            return Err(format!(
                "expectation {} q must contain visible text",
                number
            ));
        }
        if contains_line_break(&expectation.a) {
            return Err(format!(
                "expectation {} expected answer must be single-line",
                number
            ));
        }
        if !contains_visible_config_text(&expectation.a) {
            return Err(format!(
                "expectation {} expected answer must contain visible text",
                number
            ));
        }
        if matches!(expectation.a.as_str(), OBSERVED_IDK | OBSERVED_MALFORMED) {
            return Err(format!(
                "expectation {} expected answer must not be idk or malformed",
                number
            ));
        }
        if let Some(cooldown) = expectation.cooldown.as_deref() {
            parse_cooldown(cooldown)
                .map_err(|err| format!("expectation {} cooldown: {}", number, err))?;
        }
        if let Some(thinking) = expectation.thinking.as_deref() {
            validate_thinking(thinking)
                .map_err(|err| format!("expectation {} thinking: {}", number, err))?;
        }
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

fn contains_visible_config_text(value: &str) -> bool {
    value
        .chars()
        .any(|char| !char.is_control() && !char.is_whitespace() && !is_invisible_format_char(char))
}

fn is_invisible_format_char(char: char) -> bool {
    // Keep this close to Unicode format-control and Default_Ignorable_Code_Point
    // ranges that can otherwise make config text look blank while still passing
    // non-empty checks. Visible text may still contain these characters; a value
    // made only from them is treated as blank.
    matches!(
        char,
        '\u{00ad}'
            | '\u{034f}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff0}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1345f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
            | '\u{e0100}'..='\u{e01ef}'
    )
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
        return Err("check.yml agent.thinking must not be empty".to_string());
    }
    if contains_line_break(value) {
        return Err("check.yml agent.thinking must be a single-line string".to_string());
    }
    match value {
        "off" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "adaptive" | "max" => {
            Ok(())
        }
        _ => Err(format!("unsupported check.yml agent.thinking: {}", value)),
    }
}

pub(crate) fn codex_reasoning_effort(thinking: &str) -> Option<&str> {
    match thinking {
        "adaptive" => None,
        "off" | "none" => Some("none"),
        "max" => Some("xhigh"),
        value => Some(value),
    }
}

pub(crate) fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
}

pub(crate) fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    normalize_repo_path(value)
        .map(|_| ())
        .map_err(|err| format!("{}: {}", label, err))
}

pub(crate) fn normalize_agent_ignore_pattern_for_config(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("agent ignore pattern: path must not be empty".to_string());
    }
    normalize_repo_path(value).map_err(|err| format!("agent ignore pattern: {}", err))
}
