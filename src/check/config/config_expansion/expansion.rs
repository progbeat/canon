use super::presets::{apply_expectation_settings, raw_presets_from_config, resolve_presets};
use super::source::CheckConfigSource;
use crate::check::config::validation::parse_cooldown_config;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, ExpectationTarget, RawCheckConfig,
    RawExpectationCommonConfig, RawExpectationFields, RawExpectationItem, RawExpectationItemForm,
    RawExpectationSettings, RawGeneratorExpectation, RawIncludeExpectation, ResolvedPresetConfig,
    DEFAULT_DIFF_FROM,
};
use crate::repo_inspection::RepoInspectionCache;
use minijinja::Environment;
use serde_json::json;
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
    pub(crate) ask_question: Option<&'a str>,
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
        agent,
        expectations: configured_expectations,
    } = raw;
    // Raw expansion is the only layer that consumes preset names. Command
    // execution receives the returned `CheckConfig`, which carries resolved
    // agent/expectation fields and no preset map to inspect later.
    let raw_presets = raw_presets_from_config(presets, agent)?;
    let resolved_presets = resolve_presets(raw_presets)?;
    let default_agent_preset = options.default_agent_preset.unwrap_or("default");
    let default_agent = resolved_presets
        .get(default_agent_preset)
        .map(ResolvedPresetConfig::agent_config)
        .ok_or_else(|| format!("unknown preset: {}", default_agent_preset))?;
    // `canon ask` supplies one synthetic explicit item so ordinary preset
    // resolution applies every selected field default at this boundary. Its
    // command-owned to/q/a fields remain higher precedence than the preset,
    // and configured check expectations never enter the ask config.
    let raw_expectations = match options.ask_question {
        Some(question) => vec![raw_ask_expectation(question, default_agent_preset)],
        None => configured_expectations,
    };
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
        expectations,
    })
}

fn raw_ask_expectation(question: &str, preset: &str) -> RawExpectationItem {
    RawExpectationItem::Unresolved(RawExpectationFields {
        explicit_q: Some(question.to_string()),
        q_template: None,
        a: Some(String::new()),
        glob: None,
        include: None,
        common: RawExpectationCommonConfig {
            to: Some(crate::config_types::ExpectationTo::Agent),
            settings: RawExpectationSettings {
                preset: Some(preset.to_string()),
                ..RawExpectationSettings::default()
            },
            ..RawExpectationCommonConfig::default()
        },
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
                        to,
                        rank,
                        settings,
                    } = common;
                    let question_context = resolved_question_context(question_context);
                    // Preserve configured presence as the canonical `Option`.
                    // Selection applies the implementation default before the
                    // evaluator path resolves the literal value to a Git tree.
                    let target = resolve_expectation_target(target)
                        .map_err(|err| format!("expectation {} target: {}", item_number, err))?;
                    let cooldown = cooldown
                        .as_ref()
                        .map(parse_cooldown_config)
                        .transpose()
                        .map_err(|err| format!("expectation {} cooldown: {}", item_number, err))?;
                    let agent = self.resolve_expectation_agent(&settings)?;
                    self.expectations.push(Expectation {
                        to: to.unwrap_or_default(),
                        q: item.q,
                        a: item.a,
                        rank: rank.unwrap_or_default(),
                        question_context,
                        diff_from,
                        target,
                        question_answer_only,
                        agent,
                        cooldown,
                    })
                }
                RawExpectationItem::Generator(item) => {
                    // Generator items are the `glob` + `q_template` + `a`
                    // expectation form from the Expectations spec.
                    self.expand_glob_generator(config_path, index, item)?
                }
                RawExpectationItem::Include(item) => {
                    self.expand_include(config_path, index, item)?
                }
                RawExpectationItem::Unresolved(_) => unreachable!("resolved item is classified"),
            }
        }
        Ok(())
    }

    fn expand_glob_generator(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawGeneratorExpectation,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let files = self.expand_globs(config_path, &item.glob, item_number, "glob")?;
        let common = self.resolve_raw_expectation_common(item.common)?;
        let target = resolve_expectation_target(common.target.clone())
            .map_err(|err| format!("expectation {} target: {}", item_number, err))?;
        let cooldown = common
            .cooldown
            .as_ref()
            .map(parse_cooldown_config)
            .transpose()
            .map_err(|err| format!("expectation {} cooldown: {}", item_number, err))?;
        // Keep configured presence and the literal selection together until
        // selection applies the implementation default.
        for file in files {
            let content = self.read_expanded_file(&file)?;
            let mut environment = Environment::new();
            environment.set_keep_trailing_newline(true);
            let readable_path = file.clone();
            environment.add_function("read", move |requested: String| {
                if requested == readable_path {
                    Ok(content.clone())
                } else {
                    Err(minijinja::Error::new(
                        minijinja::ErrorKind::InvalidOperation,
                        format!("q_template may read only its current path: {requested}"),
                    ))
                }
            });
            let template = environment
                .template_from_str(&item.q.q_template)
                .map_err(|err| format!("expectation {} q_template: {}", item_number, err))?;
            let rendered_item_q = template
                .render(json!({ "path": file }))
                .map_err(|err| format!("expectation {} q_template: {}", item_number, err))?;
            self.expectations.push(Expectation {
                to: common.to.unwrap_or_default(),
                q: rendered_item_q,
                a: item.a.clone(),
                rank: common.rank.unwrap_or_default(),
                question_context: resolved_question_context(common.question_context.clone()),
                diff_from: common.diff_from.clone(),
                target: target.clone(),
                question_answer_only: false,
                agent: self.resolve_expectation_agent(&common.settings)?,
                cooldown,
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
        let files = self.expand_globs(config_path, &item.include, item_number, "include")?;
        for file in files {
            if self.include_stack.contains(&file) {
                return Err(format!("recursive expectation include: {}", file));
            }
            self.include_stack.push(file.clone());
            let result = (|| {
                let content = self.read_expanded_file(&file)?;
                let mut included = self.parse_included_items(&file, &content)?;
                let generated_item_defaults = &item.generated_item_defaults;
                for included_item in &mut included {
                    let RawExpectationItem::Unresolved(fields) = included_item else {
                        unreachable!("included items are raw until include generator fields merge");
                    };
                    merge_declared_form_compatible_raw_expectation_field_defaults(
                        fields,
                        generated_item_defaults,
                    );
                }
                self.expand_items(Path::new(&file), included)
            })();
            self.include_stack.pop();
            result?;
        }
        Ok(())
    }

    fn expand_globs(
        &mut self,
        config_path: &Path,
        glob: &str,
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
            Some(cache) => cache.generator_paths(root, config_path, glob, &self.source)?,
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
        match fields.declared_item_form() {
            Some(RawExpectationItemForm::Include) => {
                apply_raw_expansion_item_preset_defaults(&mut fields, preset);
                RawExpectationItem::include_from_fields(fields)
            }
            Some(RawExpectationItemForm::Generator) => {
                apply_raw_expansion_item_preset_defaults(&mut fields, preset);
                RawExpectationItem::generator_from_fields(fields)
            }
            Some(RawExpectationItemForm::Explicit) => {
                apply_raw_expansion_item_preset_defaults(&mut fields, preset);
                RawExpectationItem::explicit_from_fields(fields)
            }
            None => {
                apply_raw_expansion_item_preset_defaults(&mut fields, preset);
                RawExpectationItem::from_resolved_fields(fields)
            }
        }
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
    if common.to.is_none() {
        common.to = defaults.to;
    }
    if common.rank.is_none() {
        common.rank = defaults.rank;
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

pub(super) fn merge_declared_form_compatible_raw_expectation_field_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    let declared_form = fields.declared_item_form();
    merge_raw_expectation_value_defaults(fields, defaults);
    if !matches!(declared_form, Some(RawExpectationItemForm::Generator)) {
        merge_raw_expectation_explicit_q_default(fields, defaults);
    }
    if !matches!(declared_form, Some(RawExpectationItemForm::Explicit)) {
        merge_raw_expectation_generator_shape_defaults(fields, defaults);
    }
}

fn merge_all_raw_expectation_field_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    merge_raw_expectation_value_defaults(fields, defaults);
    merge_raw_expectation_explicit_q_default(fields, defaults);
    merge_raw_expectation_generator_shape_defaults(fields, defaults);
    merge_raw_expectation_include_shape_default(fields, defaults);
}

fn merge_raw_expectation_value_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.a.is_none() {
        fields.a = defaults.a.clone();
    }
    if fields.common.settings.preset.is_none() {
        fields.common.settings.preset = defaults.common.settings.preset.clone();
    }
    merge_raw_expectation_common_defaults(&mut fields.common, &defaults.common);
}

fn merge_raw_expectation_explicit_q_default(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.explicit_q.is_none() {
        fields.explicit_q = defaults.explicit_q.clone();
    }
}

fn merge_raw_expectation_generator_shape_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.q_template.is_none() {
        fields.q_template = defaults.q_template.clone();
    }
    if fields.glob.is_none() {
        fields.glob = defaults.glob.clone();
    }
}

fn merge_raw_expectation_include_shape_default(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    if fields.include.is_none() {
        fields.include = defaults.include.clone();
    }
}

fn apply_raw_expansion_item_preset_defaults(
    fields: &mut RawExpectationFields,
    preset: &ResolvedPresetConfig,
) {
    let defaults = raw_expectation_fields_from_preset(preset);
    merge_all_raw_expectation_field_defaults(fields, &defaults);
}

fn raw_expectation_fields_from_preset(preset: &ResolvedPresetConfig) -> RawExpectationFields {
    RawExpectationFields {
        // Preset `q` is the explicit-form question default. A glob generator's
        // generated q is resolved from the item/preset `q_template`.
        explicit_q: preset.q.clone(),
        q_template: preset.q_template.clone(),
        a: preset.a.clone(),
        glob: preset.glob.clone(),
        include: preset.include.clone(),
        common: preset.common.clone(),
    }
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
