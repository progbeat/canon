use super::generator::render_generator_expectation_question;
use super::include::inherit_include_fields;
use super::presets::{apply_expectation_settings, raw_presets_from_config, resolve_presets};
use super::source::CheckConfigSource;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, RawCheckConfig, RawExpectationItem,
    RawExpectationSettings, RawGeneratorExpectation, RawIncludeExpectation,
};
use crate::repo_inspection::RepoInspectionCache;
use std::collections::BTreeMap;
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
                        agent,
                        cooldown: item.cooldown,
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
        let uses_content = item.generated_question_format.contains("{{content}}");
        for file in files {
            let content = if uses_content {
                self.read_expanded_file(&file)?
            } else {
                String::new()
            };
            self.expectations.push(Expectation {
                q: render_generator_expectation_question(&item.generated_question_format, &content),
                a: item.a.clone(),
                agent: self.resolve_expectation_agent(&item.settings)?,
                cooldown: item.cooldown.clone(),
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
