use crate::check::validation::normalize_agent_ignore_pattern_for_config;
use crate::config_types::{
    AgentConfig, CheckConfig, CooldownConfig, Expectation, RawCheckConfig, RawExpectationItem,
    RawExpectationSettings, RawGeneratorExpectation, RawIncludeExpectation, RawPresetConfig,
};
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

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
}

impl CheckConfigSource {
    fn tree_source(&self) -> &TreeSource {
        match self {
            CheckConfigSource::Tree(source) => source,
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
        let uses_content = item.question_format.contains("{{content}}");
        for file in files {
            let content = if uses_content {
                self.read_expanded_file(&file)?
            } else {
                String::new()
            };
            self.expectations.push(Expectation {
                q: render_generator_expectation_question(&item.question_format, &content),
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
                inherit_include_fields(&mut included, &item.settings, &item.cooldown);
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
        let source = self.source.tree_source().clone();
        let files = match self.cache.as_deref_mut() {
            Some(cache) => cache.generator_paths(root, config_path, path, &source)?,
            None => return Err("tree config expansion requires RepoInspectionCache".to_string()),
        };
        Ok(files)
    }

    fn read_expanded_file(&mut self, file: &str) -> Result<String, String> {
        let root = self
            .root
            .ok_or_else(|| "config expansion has no project root".to_string())?;
        match self.cache.as_deref_mut() {
            Some(cache) => cache.tree_file_content(root, self.source.tree_source(), file),
            None => Err("staged config expansion requires RepoInspectionCache".to_string()),
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

fn inherit_include_fields(
    items: &mut [RawExpectationItem],
    inherited_settings: &RawExpectationSettings,
    inherited_cooldown: &Option<CooldownConfig>,
) {
    for item in items {
        match item {
            RawExpectationItem::Explicit(item) => {
                inherit_expectation_settings(&mut item.settings, inherited_settings);
                inherit_expectation_cooldown(&mut item.cooldown, inherited_cooldown);
            }
            RawExpectationItem::Generator(item) => {
                inherit_expectation_settings(&mut item.settings, inherited_settings);
                inherit_expectation_cooldown(&mut item.cooldown, inherited_cooldown);
            }
            RawExpectationItem::Include(item) => {
                inherit_expectation_settings(&mut item.settings, inherited_settings);
                inherit_expectation_cooldown(&mut item.cooldown, inherited_cooldown);
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

fn inherit_expectation_cooldown(
    cooldown: &mut Option<CooldownConfig>,
    inherited: &Option<CooldownConfig>,
) {
    if cooldown.is_none() {
        *cooldown = inherited.clone();
    }
}

fn render_generator_expectation_question(question_format: &str, content: &str) -> String {
    // The expectations spec defines q_template rendering as plain
    // `{{content}}` substitution to produce user-authored expectation
    // questions. This is deliberately separate from Canon-owned evaluator
    // prompt/instruction templates, which are loaded only by
    // `evaluator::prompt` from `resources/prompts/`.
    question_format.replace("{{content}}", content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn include_cooldown_is_inherited_without_overriding_child_cooldown() {
        let root = test_root("include-cooldown-inheritance");
        git(&root, &["init"]);
        fs::create_dir_all(root.join("expects")).unwrap();
        fs::write(
            root.join("expects/included.yml"),
            r#"
- q: "Does the include cooldown apply?"
  a: "yes"
- q: "Does the child cooldown win?"
  a: "yes"
  cooldown: 1d
"#,
        )
        .unwrap();
        git(&root, &["add", "expects/included.yml"]);
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - include: "expects/*.yml"
    cooldown: 7d
"#,
        )
        .expect("parse raw check config");
        let mut cache = RepoInspectionCache::new();

        let config = expand_raw_check_config(
            Some(&root),
            Path::new("check.yml"),
            raw,
            Some(&mut cache),
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand config");

        assert_eq!(config.expectations.len(), 2);
        assert_eq!(
            config.expectations[0].cooldown,
            Some(CooldownConfig::Compact("7d".to_string()))
        );
        assert_eq!(
            config.expectations[1].cooldown,
            Some(CooldownConfig::Compact("1d".to_string()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_agent_config_still_expands_to_default_preset() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
agent:
  model:
    primary: "legacy-primary"
    fallbacks: ["legacy-fallback"]
  thinking: high
  ignore: ["tmp/**"]
expectations:
  - q: "Does the legacy agent expand?"
    a: "yes"
"#,
        )
        .expect("parse legacy raw check config");

        let config = expand_raw_check_config(
            None,
            Path::new("check.yml"),
            raw,
            None,
            CheckConfigSource::Tree(TreeSource::Staged),
        )
        .expect("expand legacy config");

        assert_eq!(
            config.agent.models,
            vec!["legacy-primary".to_string(), "legacy-fallback".to_string()]
        );
        assert_eq!(config.agent.thinking, "high");
        assert_eq!(config.agent.ignore, vec!["tmp/**".to_string()]);
        assert_eq!(config.presets.get("default"), Some(&config.agent));
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
