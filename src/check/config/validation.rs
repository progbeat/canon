use crate::check::core::{
    contains_line_break, is_line_break_char, matches_answer_pattern, ANSWER_PATTERN,
    ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW, INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::run::selection::{minimal_unique_expectation_prefix, parse_cooldown};
use crate::config_types::{AgentConfig, CheckConfig, Expectation, ResolvedPresetConfig};
use crate::hash::expectation_id;
use crate::logs::push_json_control_escape;
use crate::scope::normalize_repo_path;
use std::collections::BTreeSet;

pub(crate) fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if !config.presets.contains_key("default") {
        return Err("check.yml presets must contain default".to_string());
    }
    for (name, preset) in &config.presets {
        validate_agent_config(&preset.agent_config(), &format!("presets.{}", name))?;
    }
    validate_agent_config(&config.agent, "presets.default")?;
    if config.expectations.is_empty() {
        return Err("check.yml expectations must not be empty".to_string());
    }
    let ids = expectation_ids(config);
    validate_unique_expectation_ids(&ids)?;
    let display_ids = expectation_display_ids(&ids);
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        if !contains_visible_config_text(&expectation.q) {
            return Err(format!(
                "expectation {} q must contain visible text",
                number
            ));
        }
        validate_expected_answer_matches_interrogation_response_schema_answer_pattern(
            &display_ids[index],
            expectation,
        )?;
        // Expected answers cannot collide with either evaluator schema errors
        // or Canon's internal unparsable-response marker.
        if matches!(
            expectation.a.as_str(),
            ERROR_SCOPE_TOO_NARROW | ERROR_INVALID_QUESTION | INTERNAL_ERROR_UNPARSABLE
        ) {
            return Err(render_expectation_validation_error(
                &display_ids[index],
                &expectation.q,
                "expected answer must not be an evaluator error token",
                &format!(
                    "configured expected answer is `{}`",
                    escape_config_error_block_text(&expectation.a)
                ),
            ));
        }
        if let Some(cooldown) = expectation.cooldown.as_ref() {
            parse_cooldown(cooldown)
                .map_err(|err| format!("expectation {} cooldown: {}", number, err))?;
        }
        validate_agent_config(&expectation.agent, &format!("expectation {}", number))?;
    }
    Ok(())
}

fn validate_expected_answer_matches_interrogation_response_schema_answer_pattern(
    display_id: &str,
    expectation: &Expectation,
) -> Result<(), String> {
    if matches_answer_pattern(&expectation.a) {
        return Ok(());
    }
    Err(render_expectation_validation_error(
        display_id,
        &expectation.q,
        "invalid-expected-answer",
        &format!(
            "configured expected answer `{}` does not match answer pattern {}",
            escape_config_error_block_text(&expectation.a),
            ANSWER_PATTERN
        ),
    ))
}

fn render_expectation_validation_error(
    display_id: &str,
    question: &str,
    error: &str,
    evidence: &str,
) -> String {
    format!(
        "{}. ERROR\n{}\nError: {}\nEvidence: {}",
        display_id,
        escape_config_error_block_text(question),
        error,
        evidence
    )
}

fn expectation_ids(config: &CheckConfig) -> Vec<String> {
    config
        .expectations
        .iter()
        .map(|expectation| {
            expectation_id(&expectation.q, &expectation.a, &expectation.instructions)
        })
        .collect()
}

fn validate_unique_expectation_ids(ids: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            return Err(format!("duplicate expectation ID: {}", id));
        }
    }
    Ok(())
}

fn expectation_display_ids(ids: &[String]) -> Vec<String> {
    ids.iter()
        .map(|id| minimal_unique_expectation_prefix(id, ids).unwrap_or_else(|| id.clone()))
        .collect()
}

fn escape_config_error_block_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if is_line_break_char(ch) || ch.is_control() => {
                push_config_error_unicode_escape(&mut output, ch);
            }
            ch => output.push(ch),
        }
    }
    output
}

fn push_config_error_unicode_escape(output: &mut String, ch: char) {
    if (ch as u32) <= 0xff {
        push_json_control_escape(output, ch as u8);
    } else {
        let mut units = [0; 2];
        for unit in ch.encode_utf16(&mut units) {
            output.push_str(&format!("\\u{unit:04x}"));
        }
    }
}

fn validate_agent_config(agent: &AgentConfig, label: &str) -> Result<(), String> {
    for (index, model) in agent.models.iter().enumerate() {
        validate_optional_model(
            Some(model.as_str()),
            &format!("{}.models[{}]", label, index),
        )?;
    }
    validate_thinking(&agent.thinking).map_err(|err| format!("{}: {}", label, err))?;
    for path in &agent.ignore {
        normalize_agent_ignore_pattern_for_config(path)?;
    }
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
        return Err("thinking must not be empty".to_string());
    }
    if contains_line_break(value) {
        return Err("thinking must be a single-line string".to_string());
    }
    match value {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(()),
        _ => Err(format!("unsupported thinking: {}", value)),
    }
}

pub(crate) fn codex_reasoning_effort(thinking: &str) -> Option<&str> {
    Some(thinking)
}

pub(crate) fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
        || config
            .expectations
            .iter()
            .any(|expectation| !expectation.agent.plugins.is_empty())
        || config.presets.values().any(resolved_preset_loads_plugins)
}

fn resolved_preset_loads_plugins(preset: &ResolvedPresetConfig) -> bool {
    preset
        .common
        .settings
        .plugins
        .as_ref()
        .is_some_and(|plugins| !plugins.is_empty())
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

#[cfg(test)]
mod tests {
    use super::push_config_error_unicode_escape;
    use super::validate_check_config;
    use crate::config_types::{
        AgentConfig, CheckConfig, Expectation, ExpectationTarget, ResolvedPresetConfig,
    };
    use std::collections::BTreeMap;

    #[test]
    fn invalid_expected_answer_error_uses_expectation_block_format() {
        let question = "What is this project implemented in?";
        let agent = AgentConfig::default();
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), preset(&agent));
        let config = CheckConfig {
            version: 1,
            presets,
            agent: agent.clone(),
            expectations: vec![Expectation {
                q: question.to_string(),
                a: "Rust".to_string(),
                instructions: String::new(),
                diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
                target: None,
                question_answer_only: false,
                agent,
                cooldown: None,
            }],
        };

        let error = validate_check_config(&config).unwrap_err();

        let mut lines = error.lines();
        let header = lines.next().unwrap();
        assert!(header.ends_with(". ERROR"));
        assert_ne!(header, ". ERROR");
        assert_eq!(lines.next(), Some(question));
        assert_eq!(lines.next(), Some("Error: invalid-expected-answer"));
        assert_eq!(
            lines.next(),
            Some("Evidence: configured expected answer `Rust` does not match answer pattern ^[-_a-z0-9]+$")
        );
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn duplicate_expectation_ids_are_rejected_even_when_targets_differ() {
        let agent = AgentConfig::default();
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), preset(&agent));
        let expectation = |target| Expectation {
            q: "Does this behavior work?".to_string(),
            a: "yes".to_string(),
            instructions: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target,
            question_answer_only: false,
            agent: agent.clone(),
            cooldown: None,
        };
        let config = CheckConfig {
            version: 1,
            presets,
            agent: agent.clone(),
            expectations: vec![
                expectation(None),
                expectation(Some(ExpectationTarget::Diff)),
            ],
        };

        let error = validate_check_config(&config).unwrap_err();

        assert!(error.starts_with("duplicate expectation ID: "), "{error}");
    }

    #[test]
    fn unicode_escape_uses_surrogate_pairs_for_non_bmp_codepoints() {
        let mut escaped = String::new();

        push_config_error_unicode_escape(&mut escaped, '\u{1f600}');

        assert_eq!(escaped, "\\ud83d\\ude00");
    }

    fn preset(agent: &AgentConfig) -> ResolvedPresetConfig {
        let mut preset = ResolvedPresetConfig::default();
        preset.common.settings.models = Some(agent.models.clone());
        preset.common.settings.thinking = Some(agent.thinking.clone());
        preset.common.settings.ignore = Some(agent.ignore.clone());
        preset.common.settings.plugins = Some(agent.plugins.clone());
        preset
    }
}
