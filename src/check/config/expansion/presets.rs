//! Resolves named preset inheritance and expectation field defaults.

use crate::check::config::validation::{
    normalize_agent_ignore_pattern_for_config, validate_agent_config,
};
use crate::config_types::{
    AgentConfig, ConfiguredValue, RawExpectationSettings, RawLegacyAgentConfig, RawPresetConfig,
    ResolvedPresetConfig,
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

pub(super) fn apply_expectation_settings(
    agent: &mut AgentConfig,
    settings: &RawExpectationSettings,
) -> Result<(), String> {
    if let Some(models) = &settings.models.value {
        agent.models = models.clone();
    }
    if let Some(thinking) = &settings.thinking.value {
        agent.thinking = thinking.clone();
    }
    agent.ignore = settings.ignore.value.clone();
    agent.ignore_configured = settings.ignore.configured;
    if let Some(plugins) = &settings.plugins.value {
        agent.plugins = plugins.clone();
    }
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
    let common = &mut preset.common;
    if raw.question_context.configured {
        common.question_context = raw.question_context.clone();
    }
    if raw.diff_from.configured {
        common.git_backed.diff_from = raw.diff_from.clone();
    }
    if raw.target.configured {
        common.git_backed.target = raw.target.clone();
    }
    if raw.cooldown.configured {
        common.git_backed.cooldown = raw.cooldown.clone();
    }
    if raw.to.configured {
        common.to = raw.to.clone();
    }
    if raw.rank.configured {
        common.rank = raw.rank.clone();
    }
    if raw.models.configured {
        common.settings.models = raw.models.clone();
    }
    if raw.thinking.configured {
        common.settings.thinking = raw.thinking.clone();
    }
    if raw.ignore.configured {
        common.settings.ignore = raw.ignore.clone();
    }
    if raw.plugins.configured {
        common.settings.plugins = raw.plugins.clone();
    }
}

fn normalize_preset_config(name: &str, preset: &mut ResolvedPresetConfig) -> Result<(), String> {
    if let Some(ignore) = &mut preset.common.settings.ignore.value {
        for pattern in ignore {
            *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
        }
    }
    validate_agent_config(&preset.agent_config(), &format!("presets.{}", name))?;
    Ok(())
}

fn normalize_agent_config(agent: &mut AgentConfig) -> Result<(), String> {
    if let Some(ignore) = &mut agent.ignore {
        for pattern in ignore {
            *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
        }
    }
    Ok(())
}
