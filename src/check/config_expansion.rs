use crate::check::generator_paths::expand_generator_paths;
use crate::check::validation::normalize_agent_ignore_pattern_for_config;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, RawCheckConfig, RawExpectationItem,
    RawExpectationSettings, RawGeneratorExpectation, RawIncludeExpectation, RawPresetConfig,
};
use crate::git::tree_source::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[cfg(test)]
use std::fs;

pub(crate) fn expand_raw_check_config(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    cache: Option<&mut RepoInspectionCache>,
    source: CheckConfigSource,
) -> Result<CheckConfig, String> {
    let raw_presets = raw_presets_from_config(raw.presets, raw.agent)?;
    let presets = resolve_presets(raw_presets)?;
    let default_agent = presets
        .get("default")
        .cloned()
        .ok_or_else(|| "check.yml presets must contain default".to_string())?;
    let expectations = {
        let mut expansion = RawExpectationExpansion {
            root,
            cache,
            source,
            presets: &presets,
            include_stack: Vec::new(),
            expectations: Vec::new(),
        };
        expansion.expand_items(config_path, raw.expectations)?;
        expansion.expectations
    };
    Ok(CheckConfig {
        version: raw.version,
        presets,
        agent: default_agent,
        expectations,
    })
}

fn raw_presets_from_config(
    presets: Option<BTreeMap<String, RawPresetConfig>>,
    legacy_agent: Option<crate::config_types::RawLegacyAgentConfig>,
) -> Result<BTreeMap<String, RawPresetConfig>, String> {
    match (presets, legacy_agent) {
        (Some(presets), None) => Ok(presets),
        (None, Some(agent)) => {
            let mut presets = BTreeMap::new();
            presets.insert("default".to_string(), raw_preset_from_legacy_agent(agent));
            Ok(presets)
        }
        (Some(_), Some(_)) => Err("check.yml must not contain both presets and agent".to_string()),
        (None, None) => Err("check.yml presets must contain default".to_string()),
    }
}

fn raw_preset_from_legacy_agent(
    agent: crate::config_types::RawLegacyAgentConfig,
) -> RawPresetConfig {
    let mut models = Vec::new();
    if let Some(primary) = agent.model.primary {
        models.push(primary);
    }
    models.extend(agent.model.fallbacks);
    RawPresetConfig {
        extends: None,
        models: (!models.is_empty()).then_some(models),
        thinking: agent.thinking,
        ignore: agent.ignore,
        plugins: agent.plugins,
    }
}

fn normalize_agent_config(mut agent: AgentConfig) -> Result<AgentConfig, String> {
    for pattern in &mut agent.ignore {
        *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
    }
    Ok(agent)
}

fn resolve_presets(
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
    let mut agent = if let Some(parent) = raw.extends.as_deref() {
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

fn apply_expectation_settings(
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

#[derive(Clone)]
pub(crate) enum CheckConfigSource {
    Tree(TreeSource),
    #[cfg(test)]
    Worktree,
}

impl CheckConfigSource {
    fn tree_source(&self) -> Option<&TreeSource> {
        match self {
            CheckConfigSource::Tree(source) => Some(source),
            #[cfg(test)]
            CheckConfigSource::Worktree => None,
        }
    }
}

struct RawExpectationExpansion<'a> {
    root: Option<&'a Path>,
    cache: Option<&'a mut RepoInspectionCache>,
    source: CheckConfigSource,
    presets: &'a BTreeMap<String, AgentConfig>,
    include_stack: Vec<String>,
    expectations: Vec<Expectation>,
}

impl RawExpectationExpansion<'_> {
    fn expand_items(
        &mut self,
        config_path: &Path,
        items: Vec<RawExpectationItem>,
    ) -> Result<(), String> {
        for (index, item) in items.into_iter().enumerate() {
            match item {
                RawExpectationItem::Explicit(item) => {
                    let agent = self.resolve_expectation_agent(&item.settings)?;
                    self.expectations.push(Expectation {
                        q: item.q,
                        a: item.a,
                        prompt_scope: Vec::new(),
                        agent,
                        cooldown: item.cooldown,
                        thinking: item.settings.thinking,
                    })
                }
                RawExpectationItem::Generator(item) => {
                    self.expand_path_generator(config_path, index, item)?
                }
                RawExpectationItem::Include(item) => {
                    self.expand_include(config_path, index, item)?
                }
            }
        }
        Ok(())
    }

    fn expand_path_generator(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawGeneratorExpectation,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let files = self.expand_paths(config_path, &item.path, item_number, "path")?;
        let uses_content = item.question_template.contains("{{content}}");
        for file in files {
            let content = if uses_content {
                self.read_expanded_file(&file)?
            } else {
                String::new()
            };
            self.expectations.push(Expectation {
                q: render_generator_question(&item.question_template, &content),
                a: item.a.clone(),
                prompt_scope: if uses_content { vec![file] } else { Vec::new() },
                agent: self.resolve_expectation_agent(&item.settings)?,
                cooldown: item.cooldown.clone(),
                thinking: item.settings.thinking.clone(),
            });
        }
        Ok(())
    }

    fn expand_include(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawIncludeExpectation,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let files = self.expand_paths(config_path, &item.include, item_number, "include")?;
        for file in files {
            if self.include_stack.contains(&file) {
                return Err(format!("recursive expectation include: {}", file));
            }
            self.include_stack.push(file.clone());
            let result = (|| {
                let content = self.read_expanded_file(&file)?;
                let mut included = self.parse_included_items(&file, &content)?;
                inherit_include_settings(&mut included, &item.settings);
                self.expand_items(Path::new(&file), included)
            })();
            self.include_stack.pop();
            result?;
        }
        Ok(())
    }

    fn expand_paths(
        &mut self,
        config_path: &Path,
        path: &str,
        item_number: usize,
        label: &str,
    ) -> Result<Vec<String>, String> {
        let root = self.root.ok_or_else(|| {
            format!(
                "expectation {} uses {} but config expansion has no project root",
                item_number, label
            )
        })?;
        let files = if let Some(source) = self.source.tree_source().cloned() {
            match self.cache.as_deref_mut() {
                Some(cache) => cache.generator_paths(root, config_path, path, &source)?,
                None => {
                    return Err("tree config expansion requires RepoInspectionCache".to_string())
                }
            }
        } else {
            expand_generator_paths(root, config_path, path, false)?
        };
        Ok(files)
    }

    fn read_expanded_file(&mut self, file: &str) -> Result<String, String> {
        let root = self
            .root
            .ok_or_else(|| "config expansion has no project root".to_string())?;
        match self.source {
            CheckConfigSource::Tree(ref source) => match self.cache.as_deref_mut() {
                Some(cache) => cache.tree_file_content(root, source, file),
                None => Err("staged config expansion requires RepoInspectionCache".to_string()),
            },
            #[cfg(test)]
            CheckConfigSource::Worktree => {
                let absolute = root.join(file);
                match self.cache.as_deref_mut() {
                    Some(cache) => cache.read_to_string(&absolute),
                    None => fs::read_to_string(&absolute)
                        .map_err(|err| format!("failed to read {}: {}", file, err)),
                }
            }
        }
    }

    fn parse_included_items(
        &mut self,
        file: &str,
        content: &str,
    ) -> Result<Vec<RawExpectationItem>, String> {
        match self.cache.as_deref_mut() {
            Some(cache) => cache.included_expectation_items(file, content),
            None => serde_saphyr::from_str(content)
                .map_err(|err| format!("failed to parse {}: {}", file, err)),
        }
    }

    fn resolve_expectation_agent(
        &self,
        settings: &RawExpectationSettings,
    ) -> Result<AgentConfig, String> {
        let preset = settings.preset.as_deref().unwrap_or("default");
        let mut agent = self
            .presets
            .get(preset)
            .cloned()
            .ok_or_else(|| format!("unknown preset: {}", preset))?;
        apply_expectation_settings(&mut agent, settings)?;
        Ok(agent)
    }
}

fn inherit_include_settings(items: &mut [RawExpectationItem], inherited: &RawExpectationSettings) {
    for item in items {
        match item {
            RawExpectationItem::Explicit(item) => {
                inherit_expectation_settings(&mut item.settings, inherited);
            }
            RawExpectationItem::Generator(item) => {
                inherit_expectation_settings(&mut item.settings, inherited);
            }
            RawExpectationItem::Include(item) => {
                inherit_expectation_settings(&mut item.settings, inherited);
            }
        }
    }
}

fn inherit_expectation_settings(
    settings: &mut RawExpectationSettings,
    inherited: &RawExpectationSettings,
) {
    if settings.preset.is_none() {
        settings.preset = inherited.preset.clone();
    }
    if settings.models.is_none() {
        settings.models = inherited.models.clone();
    }
    if settings.thinking.is_none() {
        settings.thinking = inherited.thinking.clone();
    }
    if settings.ignore.is_none() {
        settings.ignore = inherited.ignore.clone();
    }
    if settings.plugins.is_none() {
        settings.plugins = inherited.plugins.clone();
    }
}

fn render_generator_question(template: &str, content: &str) -> String {
    // The expectations spec defines q_template rendering as plain
    // `{{content}}` substitution to produce user-authored expectation
    // questions. This is deliberately separate from Canon-owned evaluator
    // prompt/instruction templates, which are loaded only by
    // `evaluator::prompt` from `resources/prompts/`.
    template.replace("{{content}}", content)
}
