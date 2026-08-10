//! Resolves named preset inheritance and expectation field defaults.

use crate::check::config::validation::{
    normalize_agent_ignore_pattern_for_config, validate_resolved_agent_config,
};
use crate::config_types::{
    preset_names_in_precedence_order, AgentConfig, ConfiguredValue, RawExpectationSettings,
    RawLegacyAgentConfig, RawPresetConfig, ResolvedPresetConfig,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn raw_presets_from_config(
    presets: Option<BTreeMap<String, RawPresetConfig>>,
    legacy_agent: Option<RawLegacyAgentConfig>,
) -> Result<BTreeMap<String, RawPresetConfig>, String> {
    match (presets, legacy_agent) {
        (Some(presets), None) => Ok(presets),
        (None, Some(agent)) => {
            // Backward compatibility for check.yml files written before
            // named presets: top-level `agent` still maps to `presets.default`.
            let mut presets = BTreeMap::new();
            presets.insert("default".to_string(), raw_preset_from_legacy_agent(agent));
            Ok(presets)
        }
        (Some(_), Some(_)) => Err("check.yml must not contain both presets and agent".to_string()),
        (None, None) => Err("check.yml presets must contain default".to_string()),
    }
}

pub(super) fn resolve_presets(
    raw_presets: BTreeMap<String, RawPresetConfig>,
) -> Result<BTreeMap<String, ResolvedPresetConfig>, String> {
    if !raw_presets.contains_key("default") {
        return Err("check.yml presets must contain default".to_string());
    }
    let mut resolved = BTreeMap::new();
    for name in raw_presets.keys() {
        let mut resolving = BTreeSet::new();
        let agent = resolve_preset(name, &raw_presets, &mut resolved, &mut resolving)?;
        resolved.insert(name.clone(), agent);
    }
    Ok(resolved)
}

pub(super) fn resolve_preset_closure(
    raw_presets: &BTreeMap<String, RawPresetConfig>,
    selected_presets: &str,
) -> Result<BTreeMap<String, ResolvedPresetConfig>, String> {
    if !raw_presets.contains_key("default") {
        return Err("check.yml presets must contain default".to_string());
    }
    let mut resolved = BTreeMap::new();
    let mut resolving = BTreeSet::new();
    for preset_name in preset_names_in_precedence_order(selected_presets) {
        resolve_preset(preset_name, raw_presets, &mut resolved, &mut resolving)?;
    }
    Ok(resolved)
}

pub(super) fn apply_expectation_settings(
    agent: &mut AgentConfig,
    settings: &RawExpectationSettings,
) -> Result<(), String> {
    settings.apply_to_agent(agent);
    normalize_agent_config(agent)
}

fn raw_preset_from_legacy_agent(agent: RawLegacyAgentConfig) -> RawPresetConfig {
    let mut models = Vec::new();
    if let Some(primary) = agent.model.primary {
        models.push(primary);
    }
    models.extend(agent.model.fallbacks);
    RawPresetConfig {
        q: None,
        a: None,
        to: Default::default(),
        rank: Default::default(),
        question_context: Default::default(),
        diff_from: Default::default(),
        target: Default::default(),
        cooldown: Default::default(),
        q_scope: Default::default(),
        preset: None,
        models: ConfiguredValue::from_option((!models.is_empty()).then_some(models)),
        thinking: ConfiguredValue::from_option(agent.thinking),
        ignore: agent.ignore,
        plugins: ConfiguredValue::from_option(agent.plugins),
    }
}

fn resolve_preset(
    name: &str,
    raw_presets: &BTreeMap<String, RawPresetConfig>,
    resolved: &mut BTreeMap<String, ResolvedPresetConfig>,
    resolving: &mut BTreeSet<String>,
) -> Result<ResolvedPresetConfig, String> {
    if let Some(preset) = resolved.get(name) {
        return Ok(preset.clone());
    }
    if !resolving.insert(name.to_string()) {
        return Err(format!("preset inheritance cycle includes {}", name));
    }
    let raw = raw_presets
        .get(name)
        .ok_or_else(|| format!("unknown preset: {}", name))?;
    let mut agent = if let Some(parent) = raw.preset.as_deref() {
        resolve_preset(parent, raw_presets, resolved, resolving)?
    } else {
        ResolvedPresetConfig::default()
    };
    apply_raw_preset(&mut agent, raw);
    normalize_preset_config(name, &mut agent)?;
    resolving.remove(name);
    resolved.insert(name.to_string(), agent.clone());
    Ok(agent)
}

fn apply_raw_preset(preset: &mut ResolvedPresetConfig, raw: &RawPresetConfig) {
    if let Some(q) = &raw.q {
        preset.q = Some(q.clone());
    }
    if let Some(a) = &raw.a {
        preset.a = Some(a.clone());
    }
    let mut common = raw.expectation_common();
    common.fill_missing_from(&preset.common);
    preset.common = common;
}

fn normalize_preset_config(name: &str, preset: &mut ResolvedPresetConfig) -> Result<(), String> {
    normalize_ignore_patterns(&mut preset.common.settings.ignore.value)?;
    validate_resolved_agent_config(&preset.agent_config(), &format!("presets.{}", name))?;
    Ok(())
}

fn normalize_agent_config(agent: &mut AgentConfig) -> Result<(), String> {
    normalize_ignore_patterns(&mut agent.ignore)
}

fn normalize_ignore_patterns(ignore: &mut Option<Vec<String>>) -> Result<(), String> {
    if let Some(ignore) = ignore {
        for pattern in ignore {
            *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
