use super::defaults::apply_raw_expansion_item_preset_defaults;
use super::fields::{
    resolve_expectation_target, resolve_q_scope, resolved_expectation_diff_from,
    resolved_question_context,
};
use crate::check::config::expansion::presets::apply_expectation_settings;
use crate::check::config::expansion::rank::resolve_expectation_rank;
use crate::check::config::validation::parse_cooldown_config;
use crate::config_types::{
    AgentConfig, Expectation, RawExpectationCommonConfig, RawExpectationItem,
    RawExpectationSettings, RawGitBackedExpectationConfig, ResolvedPresetConfig,
};
use std::collections::BTreeMap;

pub(super) struct RawExpectationExpansion<'a> {
    pub(super) presets: &'a BTreeMap<String, ResolvedPresetConfig>,
}

// This impl is the raw config expansion boundary. It may consume named presets;
// check execution receives only the resolved `Expectation` values it produces.
impl RawExpectationExpansion<'_> {
    pub(super) fn expand_items(
        &self,
        items: Vec<RawExpectationItem>,
    ) -> Result<Vec<Expectation>, String> {
        let mut expectations = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let item_number = index + 1;
            let item = self
                .resolve_raw_expectation_item(item)
                .map_err(|err| format!("expectation {}: {}", item_number, err))?;
            match item {
                RawExpectationItem::Explicit(item) => {
                    let common = item.common;
                    let in_place_compatibility = common.in_place_compatibility();
                    let RawExpectationCommonConfig {
                        question_context,
                        git_backed:
                            RawGitBackedExpectationConfig {
                                diff_from,
                                target,
                                cooldown,
                                q_scope,
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
                    let q_scope = resolve_q_scope(q_scope.value)
                        .map_err(|err| format!("expectation {} q-scope: {}", item_number, err))?;
                    let agent = self.resolve_expectation_agent(&settings)?;
                    expectations.push(Expectation {
                        to: to.resolved_or_implementation_default(),
                        q: item.q,
                        a: item.a,
                        rank: resolve_expectation_rank(rank.value),
                        question_context,
                        diff_from: resolved_expectation_diff_from(diff_from.value),
                        target,
                        agent,
                        cooldown,
                        q_scope,
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
        // [1H] Missing fields are filled once, so visiting the selection from
        // right to left preserves item > rightmost preset > ... > leftmost
        // preset > implementation-default precedence.
        for preset_name in preset_selection.rsplit('+').map(str::trim) {
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
