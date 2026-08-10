//! Applies only the additional `canon check --in-place` config restrictions.
//!
//! Expansion and general config validation run before this wrapper is
//! constructed. For ask, typed raw-field inspection rejects prohibited fields
//! on configured xpecs without expanding those discarded xpecs. Git-backed
//! commands use their validated config directly; in-place commands use this
//! module for the separate mode contract.

use crate::config_types::{
    preset_names_in_precedence_order, CheckConfig, Expectation, InPlaceIncompatibleField,
    RawExpectationItem, RawPresetConfig,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(crate) struct InPlaceCheckConfig {
    config: CheckConfig,
}

impl InPlaceCheckConfig {
    pub(crate) fn from_config(config: CheckConfig) -> InPlaceCheckConfig {
        InPlaceCheckConfig { config }
    }

    pub(crate) fn config(&self) -> &CheckConfig {
        &self.config
    }

    pub(crate) fn into_config(self) -> CheckConfig {
        self.config
    }

    pub(crate) fn validate_configured_fields(&self) -> Result<(), String> {
        // [m,90] Cached Result and its optional `cooldown` field apply to the
        // expectation-and-Git-state domain. The separate in-place contract has
        // no Git state: it rejects configured ignore on the effective agent and
        // Git-backed fields on every configured expectation, including
        // unselected expectations.
        // [T5] Configuration provenance survives expansion, so explicit null
        // is rejected just like every other configured ignore value.
        if self.config.agent.ignore_configured {
            return Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string(),
            );
        }
        validate_in_place_expectations(&self.config.expectations)
    }
}

pub(super) fn validate_in_place_expectations(expectations: &[Expectation]) -> Result<(), String> {
    for (index, expectation) in expectations.iter().enumerate() {
        validate_in_place_fields(
            index,
            expectation.in_place_compatibility.incompatible_fields(),
        )?;
    }
    Ok(())
}

pub(super) fn validate_raw_in_place_expectations(
    expectations: &[RawExpectationItem],
    presets: &BTreeMap<String, RawPresetConfig>,
) -> Result<(), String> {
    for (index, expectation) in expectations.iter().enumerate() {
        let common = expectation.common();
        let mut incompatible_fields = BTreeSet::new();
        let compatibility = common.in_place_compatibility();
        collect_incompatible_fields(
            &mut incompatible_fields,
            compatibility.incompatible_fields(),
        );
        let selected_presets = common.settings.preset.as_deref().unwrap_or("default");
        let mut visited_presets = BTreeSet::new();
        for preset in preset_names_in_precedence_order(selected_presets) {
            collect_preset_incompatible_fields(
                preset,
                presets,
                &mut visited_presets,
                &mut incompatible_fields,
            );
        }
        let incompatible_fields = incompatible_fields.into_iter().collect::<Vec<_>>();
        validate_in_place_fields(index, &incompatible_fields)?;
    }
    Ok(())
}

fn collect_preset_incompatible_fields(
    preset_name: &str,
    presets: &BTreeMap<String, RawPresetConfig>,
    visited: &mut BTreeSet<String>,
    incompatible_fields: &mut BTreeSet<InPlaceIncompatibleField>,
) {
    if !visited.insert(preset_name.to_string()) {
        return;
    }
    let Some(preset) = presets.get(preset_name) else {
        return;
    };
    let common = preset.expectation_common();
    let compatibility = common.in_place_compatibility();
    collect_incompatible_fields(incompatible_fields, compatibility.incompatible_fields());
    if let Some(parent) = preset.preset.as_deref() {
        collect_preset_incompatible_fields(parent, presets, visited, incompatible_fields);
    }
}

fn collect_incompatible_fields(
    fields: &mut BTreeSet<InPlaceIncompatibleField>,
    incompatible_fields: &[InPlaceIncompatibleField],
) {
    fields.extend(incompatible_fields);
}

fn validate_in_place_fields(
    index: usize,
    incompatible_fields: &[InPlaceIncompatibleField],
) -> Result<(), String> {
    if !incompatible_fields.is_empty() {
        return Err(format!(
            "expectation {} is invalid in in-place mode: {}",
            index + 1,
            incompatible_fields
                .iter()
                .map(|field| format!("`{}` requires Git-backed check state", field.config_name()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
