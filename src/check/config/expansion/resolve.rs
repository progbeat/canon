//! Resolves raw check configuration into fully expanded runtime configuration.

use super::presets::{apply_expectation_settings, raw_presets_from_config, resolve_presets};
use super::rank::resolve_expectation_rank;
use crate::check::config::in_place::validate_in_place_expectations;
use crate::check::config::validation::parse_cooldown_config;
use crate::config_types::{
    AgentConfig, CheckConfig, ConfiguredValue, Expectation, ExpectationTarget,
    InPlaceIncompatibleField, RawCheckConfig, RawExpectationCommonConfig, RawExpectationFields,
    RawExpectationItem, RawExpectationSettings, RawGitBackedExpectationConfig,
    ResolvedPresetConfig, DEFAULT_DIFF_FROM,
};
use std::collections::BTreeMap;

#[cfg(test)]
pub(crate) fn expand_raw_check_config(raw: RawCheckConfig) -> Result<CheckConfig, String> {
    expand_raw_check_config_with_options(raw, CheckConfigExpansionOptions::default())
}

#[derive(Default)]
pub(crate) struct CheckConfigExpansionOptions<'a> {
    pub(crate) default_agent_preset: Option<&'a str>,
    pub(crate) ask_question: Option<&'a str>,
    pub(crate) in_place: bool,
}

#[cfg(test)]
pub(crate) fn expand_raw_check_config_with_options(
    raw: RawCheckConfig,
    options: CheckConfigExpansionOptions<'_>,
) -> Result<CheckConfig, String> {
    expand_raw_check_config_for_command(raw, options)
}

pub(crate) fn expand_raw_check_config_for_command(
    raw: RawCheckConfig,
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
    let resolved_default_agent_preset = resolved_presets
        .get(default_agent_preset)
        .ok_or_else(|| format!("unknown preset: {}", default_agent_preset))?;
    let default_agent = resolved_default_agent_preset.agent_config();
    let expansion = RawExpectationExpansion {
        presets: &resolved_presets,
    };
    let configured_expectations = expansion.expand_items(configured_expectations)?;
    // `canon ask` supplies one synthetic explicit item so ordinary preset
    // resolution applies every selected field default at this boundary. Its
    // command-owned to/q/a fields remain higher precedence than the preset,
    // and configured check expectations never enter the ask runtime config.
    // [1r,I4,T] In-place ask validates the canonical configured expectations
    // before consuming them to construct its single runtime expectation. The
    // lossy command transformation therefore needs no retained side copy.
    if options.in_place && options.ask_question.is_some() {
        validate_in_place_expectations(&configured_expectations)?;
    }
    let runtime_expectations = match options.ask_question {
        Some(question) => {
            expansion.expand_items(vec![raw_ask_expectation(question, default_agent_preset)])?
        }
        None => configured_expectations,
    };
    Ok(CheckConfig {
        version,
        agent: default_agent,
        expectations: runtime_expectations,
    })
}

fn raw_ask_expectation(question: &str, preset: &str) -> RawExpectationItem {
    RawExpectationItem::Unresolved(RawExpectationFields {
        explicit_q: Some(question.to_string()),
        a: Some(String::new()),
        common: RawExpectationCommonConfig {
            to: ConfiguredValue::some(crate::config_types::ExpectationTo::Agent),
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
}

// This impl is the raw config expansion boundary. It may consume named presets;
// check execution receives only the resolved `Expectation` values it produces.
impl RawExpectationExpansion<'_> {
    fn expand_items(&self, items: Vec<RawExpectationItem>) -> Result<Vec<Expectation>, String> {
        let mut expectations = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let item_number = index + 1;
            let item = self
                .resolve_raw_expectation_item(item)
                .map_err(|err| format!("expectation {}: {}", item_number, err))?;
            match item {
                RawExpectationItem::Explicit(item) => {
                    let common = item.common;
                    let question_answer_only = resolved_common_is_question_answer_only(&common);
                    let mut in_place_compatibility = common.git_backed.in_place_compatibility();
                    if common.settings.ignore.configured {
                        in_place_compatibility = in_place_compatibility
                            .with_incompatible_field(InPlaceIncompatibleField::Ignore);
                    }
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
                    let question_context = resolved_question_context(question_context.value);
                    let target = resolve_expectation_target(target.value)
                        .map_err(|err| format!("expectation {} target: {}", item_number, err))?;
                    let cooldown = cooldown
                        .value
                        .as_ref()
                        .map(parse_cooldown_config)
                        .transpose()
                        .map_err(|err| format!("expectation {} cooldown: {}", item_number, err))?;
                    let agent = self.resolve_expectation_agent(&settings)?;
                    expectations.push(Expectation {
                        to: to.value.unwrap_or_default(),
                        q: item.q,
                        a: item.a,
                        rank: resolve_expectation_rank(rank.value),
                        question_context,
                        diff_from: resolved_expectation_diff_from(diff_from.value),
                        target,
                        question_answer_only,
                        agent,
                        cooldown,
                        in_place_compatibility,
                    })
                }
                RawExpectationItem::Unresolved(_) => unreachable!("resolved item is classified"),
            }
        }
        Ok(expectations)
    }

    fn resolve_expectation_agent(
        &self,
        settings: &RawExpectationSettings,
    ) -> Result<AgentConfig, String> {
        let mut agent = AgentConfig::implementation_default();
        apply_expectation_settings(&mut agent, settings)?;
        Ok(agent)
    }

    fn resolve_raw_expectation_item(
        &self,
        item: RawExpectationItem,
    ) -> Result<RawExpectationItem, String> {
        let RawExpectationItem::Unresolved(mut fields) = item else {
            return Ok(item);
        };
        let preset_selection = fields
            .common
            .settings
            .preset
            .take()
            .unwrap_or_else(|| "default".to_string());
        // [21] Missing fields are filled once, so visiting the selection from
        // right to left preserves item > rightmost preset > ... > leftmost
        // preset > implementation-default precedence.
        for preset_name in preset_selection.rsplit('+') {
            let preset = self
                .presets
                .get(preset_name)
                .ok_or_else(|| format!("unknown preset: {}", preset_name))?;
            apply_raw_expansion_item_preset_defaults(&mut fields, preset);
        }
        // [1r] The selection has been consumed into resolved field values. Do
        // not carry its names into later expansion or recover the presets.
        RawExpectationItem::from_resolved_fields(fields).map_err(str::to_string)
    }
}

fn resolved_common_is_question_answer_only(common: &RawExpectationCommonConfig) -> bool {
    common.git_backed.cooldown.value.is_none()
        && resolved_common_settings_are_empty(&common.settings)
        && resolved_question_context(common.question_context.value.clone()).is_empty()
        && resolved_expectation_diff_from(common.git_backed.diff_from.value.clone())
            == DEFAULT_DIFF_FROM
        && common.git_backed.target.value.is_none()
}

fn resolved_common_settings_are_empty(settings: &RawExpectationSettings) -> bool {
    settings.models.value.is_none()
        && settings.thinking.value.is_none()
        && settings.ignore.value.is_none()
        && settings.plugins.value.is_none()
}

pub(super) fn merge_raw_expectation_common_defaults(
    common: &mut RawExpectationCommonConfig,
    defaults: &RawExpectationCommonConfig,
) {
    if !common.question_context.configured {
        common.question_context = defaults.question_context.clone();
    }
    if !common.git_backed.diff_from.configured {
        common.git_backed.diff_from = defaults.git_backed.diff_from.clone();
    }
    if !common.git_backed.target.configured {
        common.git_backed.target = defaults.git_backed.target.clone();
    }
    if !common.git_backed.cooldown.configured {
        common.git_backed.cooldown = defaults.git_backed.cooldown.clone();
    }
    if !common.to.configured {
        common.to = defaults.to.clone();
    }
    if !common.rank.configured {
        common.rank = defaults.rank.clone();
    }
    if !common.settings.models.configured {
        common.settings.models = defaults.settings.models.clone();
    }
    if !common.settings.thinking.configured {
        common.settings.thinking = defaults.settings.thinking.clone();
    }
    if !common.settings.ignore.configured {
        common.settings.ignore = defaults.settings.ignore.clone();
    }
    if !common.settings.plugins.configured {
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
