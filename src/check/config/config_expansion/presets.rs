use crate::check::config::validation::normalize_agent_ignore_pattern_for_config;
use crate::config_types::{
    AgentConfig, RawExpectationSettings, RawLegacyAgentConfig, RawPresetConfig,
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
) -> Result<BTreeMap<String, AgentConfig>, String> {
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

pub(super) fn apply_expectation_settings(
    agent: &mut AgentConfig,
    settings: &RawExpectationSettings,
) -> Result<(), String> {
    if let Some(models) = &settings.models {
        agent.models = models.clone();
    }
    if let Some(thinking) = &settings.thinking {
        agent.thinking = thinking.clone();
    }
    if let Some(ignore) = &settings.ignore {
        agent.ignore = ignore.clone();
    }
    if let Some(plugins) = &settings.plugins {
        agent.plugins = plugins.clone();
    }
    normalize_agent_config(agent.clone()).map(|normalized| *agent = normalized)
}

fn raw_preset_from_legacy_agent(agent: RawLegacyAgentConfig) -> RawPresetConfig {
    let mut models = Vec::new();
    if let Some(primary) = agent.model.primary {
        models.push(primary);
    }
    models.extend(agent.model.fallbacks);
    RawPresetConfig {
        preset: None,
        models: (!models.is_empty()).then_some(models),
        thinking: agent.thinking,
        ignore: agent.ignore,
        plugins: agent.plugins,
    }
}

fn resolve_preset(
    name: &str,
    raw_presets: &BTreeMap<String, RawPresetConfig>,
    resolved: &mut BTreeMap<String, AgentConfig>,
    resolving: &mut BTreeSet<String>,
) -> Result<AgentConfig, String> {
    if let Some(agent) = resolved.get(name) {
        return Ok(agent.clone());
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
        AgentConfig::implementation_default()
    };
    apply_raw_preset(&mut agent, raw);
    let agent = normalize_agent_config(agent)?;
    resolving.remove(name);
    resolved.insert(name.to_string(), agent.clone());
    Ok(agent)
}

fn apply_raw_preset(agent: &mut AgentConfig, raw: &RawPresetConfig) {
    if let Some(models) = &raw.models {
        agent.models = models.clone();
    }
    if let Some(thinking) = &raw.thinking {
        agent.thinking = thinking.clone();
    }
    if let Some(ignore) = &raw.ignore {
        agent.ignore = ignore.clone();
    }
    if let Some(plugins) = &raw.plugins {
        agent.plugins = plugins.clone();
    }
}

fn normalize_agent_config(mut agent: AgentConfig) -> Result<AgentConfig, String> {
    for pattern in &mut agent.ignore {
        *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
    }
    Ok(agent)
}
