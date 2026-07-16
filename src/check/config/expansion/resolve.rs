//! Resolves raw check configuration into fully expanded runtime configuration.

use super::presets::{apply_expectation_settings, raw_presets_from_config, resolve_presets};
use super::rank::resolve_expectation_rank;
use super::source::CheckConfigSource;
use crate::check::config::validation::parse_cooldown_config;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, ExpectationTarget, RawCheckConfig,
    RawExpectationCommonConfig, RawExpectationFields, RawExpectationItem, RawExpectationSettings,
    RawGitBackedExpectationConfig, ResolvedPresetConfig, DEFAULT_DIFF_FROM,
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
    pub(crate) ask_question: Option<&'a str>,
}

#[cfg(test)]
pub(crate) fn expand_raw_check_config_with_options(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    cache: Option<&mut RepoInspectionCache>,
    source: CheckConfigSource,
    options: CheckConfigExpansionOptions<'_>,
) -> Result<CheckConfig, String> {
    Ok(
        expand_raw_check_config_with_requirements(root, config_path, raw, cache, source, options)?
            .config,
    )
}

pub(crate) fn expand_raw_check_config_with_requirements(
    _root: Option<&Path>,
    _config_path: &Path,
    raw: RawCheckConfig,
    _cache: Option<&mut RepoInspectionCache>,
    _source: CheckConfigSource,
    options: CheckConfigExpansionOptions<'_>,
) -> Result<ExpandedCheckConfig, String> {
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
    let mut expansion = RawExpectationExpansion {
        presets: &resolved_presets,
        expectations: Vec::new(),
        in_place_requirements: InPlaceRequirements {
            config_uses_ignore: !default_agent.ignore.is_empty(),
            git_backed_only_expectation_fields: Vec::new(),
        },
    };
    expansion.expand_items(raw_expectations)?;
    Ok(ExpandedCheckConfig {
        config: CheckConfig {
            version,
            agent: default_agent,
            expectations: expansion.expectations,
        },
        in_place_requirements: expansion.in_place_requirements,
    })
}

pub(crate) struct ExpandedCheckConfig {
    pub(crate) config: CheckConfig,
    pub(crate) in_place_requirements: InPlaceRequirements,
}

#[derive(Default)]
pub(crate) struct InPlaceRequirements {
    pub(crate) config_uses_ignore: bool,
    pub(crate) git_backed_only_expectation_fields: Vec<InPlaceExpectationRequirements>,
}

pub(crate) struct InPlaceExpectationRequirements {
    pub(crate) item_number: usize,
    pub(crate) git_backed_only_field_names: Vec<&'static str>,
}

fn raw_ask_expectation(question: &str, preset: &str) -> RawExpectationItem {
    RawExpectationItem::Unresolved(RawExpectationFields {
        explicit_q: Some(question.to_string()),
        a: Some(String::new()),
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
    presets: &'a BTreeMap<String, ResolvedPresetConfig>,
    expectations: Vec<Expectation>,
    in_place_requirements: InPlaceRequirements,
}

// This impl is the raw config expansion boundary. It may consume named presets;
// check execution receives only the resolved `Expectation` values it produces.
impl RawExpectationExpansion<'_> {
    fn expand_items(&mut self, items: Vec<RawExpectationItem>) -> Result<(), String> {
        for (index, item) in items.into_iter().enumerate() {
            let item_number = index + 1;
            let item = self
                .resolve_raw_expectation_item(item)
                .map_err(|err| format!("expectation {}: {}", item_number, err))?;
            // [T,Df] Preserve fields forbidden by the separate in-place
            // contract after preset resolution.
            self.in_place_requirements
                .record_expectation(item_number, &item)?;
            match item {
                RawExpectationItem::Explicit(item) => {
                    let common = self.resolve_raw_expectation_common(item.common)?;
                    let question_answer_only = resolved_common_is_question_answer_only(&common);
                    let RawExpectationCommonConfig {
                        question_context,
                        git_backed:
                            RawGitBackedExpectationConfig {
                                diff_from,
                                target,
                                cooldown,
                            },
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
                        rank: resolve_expectation_rank(rank),
                        question_context,
                        diff_from,
                        target,
                        question_answer_only,
                        agent,
                        cooldown,
                    })
                }
                RawExpectationItem::Unresolved(_) => unreachable!("resolved item is classified"),
            }
        }
        Ok(())
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
        apply_raw_expansion_item_preset_defaults(&mut fields, preset);
        RawExpectationItem::from_resolved_fields(fields).map_err(str::to_string)
    }
}

impl InPlaceRequirements {
    fn record_expectation(
        &mut self,
        item_number: usize,
        item: &RawExpectationItem,
    ) -> Result<(), String> {
        let common = match item {
            RawExpectationItem::Explicit(item) => &item.common,
            RawExpectationItem::Unresolved(_) => {
                return Err(
                    "in-place compatibility tracking requires a resolved expectation item"
                        .to_string(),
                )
            }
        };
        // [Df] The separate in-place contract rejects configuration whose
        // semantics require Git state, cached results, or path hiding.
        let mut git_backed_only_field_names = common.git_backed.configured_field_names();
        if common.settings.ignore.is_some() {
            git_backed_only_field_names.push("ignore");
        }
        if !git_backed_only_field_names.is_empty() {
            self.git_backed_only_expectation_fields
                .push(InPlaceExpectationRequirements {
                    item_number,
                    git_backed_only_field_names,
                });
        }
        Ok(())
    }
}

fn resolved_common_is_question_answer_only(common: &RawExpectationCommonConfig) -> bool {
    common.git_backed.cooldown.is_none()
        && resolved_common_settings_are_empty(&common.settings)
        && resolved_question_context(common.question_context.clone()).is_empty()
        && resolved_expectation_diff_from(common.git_backed.diff_from.clone()) == DEFAULT_DIFF_FROM
        && common.git_backed.target.is_none()
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
    if common.git_backed.diff_from.is_none() {
        common.git_backed.diff_from = defaults.git_backed.diff_from.clone();
    }
    if common.git_backed.target.is_none() {
        common.git_backed.target = defaults.git_backed.target.clone();
    }
    if common.git_backed.cooldown.is_none() {
        common.git_backed.cooldown = defaults.git_backed.cooldown.clone();
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

fn merge_all_raw_expectation_field_defaults(
    fields: &mut RawExpectationFields,
    defaults: &RawExpectationFields,
) {
    merge_raw_expectation_value_defaults(fields, defaults);
    merge_raw_expectation_explicit_q_default(fields, defaults);
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

fn apply_raw_expansion_item_preset_defaults(
    fields: &mut RawExpectationFields,
    preset: &ResolvedPresetConfig,
) {
    let defaults = raw_expectation_fields_from_preset(preset);
    merge_all_raw_expectation_field_defaults(fields, &defaults);
}

fn raw_expectation_fields_from_preset(preset: &ResolvedPresetConfig) -> RawExpectationFields {
    RawExpectationFields {
        explicit_q: preset.q.clone(),
        a: preset.a.clone(),
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
