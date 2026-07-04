use super::include::merge_include_generator_fields_as_item_fields;
use super::presets::{apply_expectation_settings, raw_presets_from_config, resolve_presets};
use super::source::CheckConfigSource;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, ExpectationTarget, RawCheckConfig,
    RawExpectationCommonConfig, RawExpectationFields, RawExpectationItem, RawExpectationSettings,
    RawGeneratorExpectation, RawIncludeExpectation, ResolvedPresetConfig, DEFAULT_DIFF_FROM,
};
use crate::repo_inspection::RepoInspectionCache;
use std::collections::BTreeMap;
use std::path::Path;

#[cfg(test)]
pub(crate) fn expand_raw_check_config(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    cache: Option<&mut RepoInspectionCache>,
    source: CheckConfigSource,
) -> Result<CheckConfig, String> {
    expand_raw_check_config_with_options(
        root,
        config_path,
        raw,
        cache,
        source,
        CheckConfigExpansionOptions::default(),
    )
}

#[derive(Default)]
pub(crate) struct CheckConfigExpansionOptions<'a> {
    pub(crate) default_agent_preset: Option<&'a str>,
}

pub(crate) fn expand_raw_check_config_with_options(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    cache: Option<&mut RepoInspectionCache>,
    source: CheckConfigSource,
    options: CheckConfigExpansionOptions<'_>,
) -> Result<CheckConfig, String> {
    let RawCheckConfig {
        version,
        presets,
        hooks,
        agent,
        expectations: raw_expectations,
    } = raw;
    // Raw expansion is the only layer that consumes preset names. Command
    // execution receives the returned `CheckConfig`, which carries resolved
    // agent/expectation fields and no preset map to inspect later.
    let raw_presets = raw_presets_from_config(presets, agent)?;
    let resolved_presets = resolve_presets(raw_presets)?;
    let hooks = hooks
        .map(crate::config_types::RawCheckHooksConfig::resolve)
        .transpose()?
        .unwrap_or_default();
    let default_agent_preset = options.default_agent_preset.unwrap_or("default");
    let default_agent = resolved_presets
        .get(default_agent_preset)
        .map(ResolvedPresetConfig::agent_config)
        .ok_or_else(|| format!("unknown preset: {}", default_agent_preset))?;
    let expectations = {
        let mut expansion = RawExpectationExpansion {
            root,
            cache,
            source,
            presets: &resolved_presets,
            include_stack: Vec::new(),
            expectations: Vec::new(),
        };
        expansion.expand_items(config_path, raw_expectations)?;
        expansion.expectations
    };
    Ok(CheckConfig {
        version,
        agent: default_agent,
        hooks,
        expectations,
    })
}

struct RawExpectationExpansion<'a> {
    root: Option<&'a Path>,
    cache: Option<&'a mut RepoInspectionCache>,
    source: CheckConfigSource,
    presets: &'a BTreeMap<String, ResolvedPresetConfig>,
    include_stack: Vec<String>,
    expectations: Vec<Expectation>,
}

// This impl is the raw config expansion boundary. It may consume named presets;
// check execution receives only the resolved `Expectation` values it produces.
impl RawExpectationExpansion<'_> {
    fn expand_items(
        &mut self,
        config_path: &Path,
        items: Vec<RawExpectationItem>,
    ) -> Result<(), String> {
        for (index, item) in items.into_iter().enumerate() {
            let item_number = index + 1;
            let item = self
                .resolve_raw_expectation_item(item)
                .map_err(|err| format!("expectation {}: {}", item_number, err))?;
            match item {
                RawExpectationItem::Explicit(item) => {
                    let common = self.resolve_raw_expectation_common(item.common)?;
                    let question_answer_only = resolved_common_is_question_answer_only(&common);
                    let RawExpectationCommonConfig {
                        question_context,
                        diff_from,
                        target,
                        cooldown,
                        settings,
                    } = common;
                    let question_context = resolved_question_context(question_context);
                    // Keep the literal `diff-from` selection here. Prompt rendering
                    // resolves it to a tree in `resolve_diff_from`, where the check
                    // runtime can validate checkpoint trees and resolve custom tree-ishs.
                    let diff_from_configured = diff_from.is_some();
                    let diff_from = resolved_expectation_diff_from(diff_from);
                    let target = resolve_expectation_target(target)
                        .map_err(|err| format!("expectation {} target: {}", item_number, err))?;
                    let agent = self.resolve_expectation_agent(&settings)?;
                    self.expectations.push(Expectation {
                        q: item.q,
                        a: item.a,
                        question_context,
                        diff_from,
                        diff_from_configured,
                        target,
                        question_answer_only,
                        agent,
                        cooldown,
                    })
                }
                RawExpectationItem::Generator(item) => {
                    // Generator items are the `path` + `q_template` + `a`
                    // expectation form from the Expectations spec.
                    self.expand_path_generator(config_path, index, item)?
                }
                RawExpectationItem::Include(item) => {
                    self.expand_include(config_path, index, item)?
                }
                RawExpectationItem::Unresolved(_) => unreachable!("resolved item is classified"),
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
        let uses_content = item.q.q_template.contains("{{content}}");
        let common = self.resolve_raw_expectation_common(item.common)?;
        let target = resolve_expectation_target(common.target.clone())
            .map_err(|err| format!("expectation {} target: {}", item_number, err))?;
        // Keep the literal `diff-from` selection here. Prompt rendering resolves
        // it to a tree in `resolve_diff_from` using the active check runtime.
        let diff_from_configured = common.diff_from.is_some();
        for file in files {
            let content = if uses_content {
                self.read_expanded_file(&file)?
            } else {
                String::new()
            };
            let rendered_item_q = item.q.q_template.replace("{{content}}", &content);
            self.expectations.push(Expectation {
                q: rendered_item_q,
                a: item.a.clone(),
                question_context: resolved_question_context(common.question_context.clone()),
                diff_from: resolved_expectation_diff_from(common.diff_from.clone()),
                diff_from_configured,
                target: target.clone(),
                question_answer_only: false,
                agent: self.resolve_expectation_agent(&common.settings)?,
                cooldown: common.cooldown.clone(),
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
                merge_include_generator_fields_as_item_fields(
                    &mut included,
                    &item.generated_item_defaults,
                );
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
        let files = match self.cache.as_deref_mut() {
            Some(cache) => cache.generator_paths(root, config_path, path, &self.source)?,
            None => return Err("tree config expansion requires RepoInspectionCache".to_string()),
        };
        Ok(files)
    }

    fn read_expanded_file(&mut self, file: &str) -> Result<String, String> {
        let root = self
            .root
            .ok_or_else(|| "config expansion has no project root".to_string())?;
        match self.cache.as_deref_mut() {
            Some(cache) => cache.config_source_file_content(root, &self.source, file),
            None => Err("config expansion requires RepoInspectionCache".to_string()),
        }
    }

    fn parse_included_items(
        &mut self,
        file: &str,
        content: &str,
    ) -> Result<Vec<RawExpectationItem>, String> {
        match self.cache.as_deref_mut() {
            Some(cache) => {
                let root = self
                    .root
                    .ok_or_else(|| "config expansion has no project root".to_string())?;
                cache.included_expectation_items(root, &self.source, file, content)
            }
            None => serde_saphyr::from_str(content)
                .map_err(|err| format!("failed to parse {}: {}", file, err)),
        }
    }

    fn resolve_expectation_agent(
        &self,
        settings: &RawExpectationSettings,
    ) -> Result<AgentConfig, String> {
        let mut agent = AgentConfig::implementation_default();
        apply_expectation_settings(&mut agent, settings)?;
        Ok(agent)
    }

    fn resolve_raw_expectation_common(
        &self,
        mut common: RawExpectationCommonConfig,
    ) -> Result<RawExpectationCommonConfig, String> {
        let preset = common.settings.preset.as_deref().unwrap_or("default");
        let preset = self
            .presets
            .get(preset)
            .ok_or_else(|| format!("unknown preset: {}", preset))?;
        merge_raw_expectation_common_defaults(&mut common, &preset.common);
        Ok(common)
    }

    fn resolve_raw_expectation_item(
        &self,
        item: RawExpectationItem,
    ) -> Result<RawExpectationItem, String> {
        let RawExpectationItem::Unresolved(mut fields) = item else {
            return Ok(item);
        };
        let preset_name = fields
            .common
            .settings
            .preset
            .as_deref()
            .unwrap_or("default");
        let preset = self
            .presets
            .get(preset_name)
            .ok_or_else(|| format!("unknown preset: {}", preset_name))?;
        // Preset defaults fill raw fields, but an item that already declares a
        // form keeps that form; preset shape fields only classify unresolved
        // items.
        let declared_form = fields.declared_item_form();
        apply_raw_expansion_item_preset_defaults(&mut fields, preset);
        let resolved_form = declared_form.or_else(|| fields.declared_item_form());
        RawExpectationItem::from_fields_with_resolved_form(fields, resolved_form)
            .map_err(str::to_string)
    }
}

fn resolved_common_is_question_answer_only(common: &RawExpectationCommonConfig) -> bool {
    common.cooldown.is_none()
        && resolved_common_settings_are_empty(&common.settings)
        && resolved_question_context(common.question_context.clone()).is_empty()
        && resolved_expectation_diff_from(common.diff_from.clone()) == DEFAULT_DIFF_FROM
        && common.target.is_none()
}

fn resolved_common_settings_are_empty(settings: &RawExpectationSettings) -> bool {
    settings.models.is_none()
        && settings.thinking.is_none()
        && settings.ignore.is_none()
        && settings.plugins.is_none()
}

pub(super) fn merge_raw_expectation_common_defaults(
    common: &mut RawExpectationCommonConfig,
    defaults: &RawExpectationCommonConfig,
) {
    if common.question_context.is_none() {
        common.question_context = defaults.question_context.clone();
    }
    if common.diff_from.is_none() {
        common.diff_from = defaults.diff_from.clone();
    }
    if common.target.is_none() {
        common.target = defaults.target.clone();
    }
    if common.cooldown.is_none() {
        common.cooldown = defaults.cooldown.clone();
    }
    if common.settings.models.is_none() {
        common.settings.models = defaults.settings.models.clone();
    }
    if common.settings.thinking.is_none() {
        common.settings.thinking = defaults.settings.thinking.clone();
    }
    if common.settings.ignore.is_none() {
        common.settings.ignore = defaults.settings.ignore.clone();
    }
    if common.settings.plugins.is_none() {
        common.settings.plugins = defaults.settings.plugins.clone();
    }
}

fn apply_raw_expansion_item_preset_defaults(
    fields: &mut RawExpectationFields,
    preset: &ResolvedPresetConfig,
) {
    // Preset `q` is the explicit-form question default. A path generator's
    // generated q is resolved from the item/preset `q_template` below.
    if fields.explicit_q.is_none() {
        fields.explicit_q = preset.q.clone();
    }
    if fields.q_template.is_none() {
        fields.q_template = preset.q_template.clone();
    }
    if fields.a.is_none() {
        fields.a = preset.a.clone();
    }
    if fields.path.is_none() {
        fields.path = preset.path.clone();
    }
    if fields.include.is_none() {
        fields.include = preset.include.clone();
    }
    merge_raw_expectation_common_defaults(&mut fields.common, &preset.common);
}

fn resolved_question_context(context: Option<String>) -> String {
    context.unwrap_or_default()
}

fn resolved_expectation_diff_from(diff_from: Option<String>) -> String {
    diff_from.unwrap_or_else(|| DEFAULT_DIFF_FROM.to_string())
}

fn resolve_expectation_target(target: Option<String>) -> Result<Option<ExpectationTarget>, String> {
    target.map(|target| target.parse()).transpose()
}
