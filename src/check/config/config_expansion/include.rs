use crate::config_types::{RawExpectationCommonConfig, RawExpectationItem};

pub(super) fn inherit_include_fields(
    items: &mut [RawExpectationItem],
    inherited: &RawExpectationCommonConfig,
) {
    for item in items {
        inherit_expectation_common_config(item.common_config_mut(), inherited);
    }
}

fn inherit_expectation_common_config(
    config: &mut RawExpectationCommonConfig,
    inherited: &RawExpectationCommonConfig,
) {
    if config.settings.preset.is_none() {
        config.settings.preset = inherited.settings.preset.clone();
    }
    if config.settings.models.is_none() {
        config.settings.models = inherited.settings.models.clone();
    }
    if config.settings.thinking.is_none() {
        config.settings.thinking = inherited.settings.thinking.clone();
    }
    if config.settings.ignore.is_none() {
        config.settings.ignore = inherited.settings.ignore.clone();
    }
    if config.settings.plugins.is_none() {
        config.settings.plugins = inherited.settings.plugins.clone();
    }
    if config.cooldown.is_none() {
        config.cooldown = inherited.cooldown.clone();
    }
    inherit_expectation_text(&mut config.instructions, &inherited.instructions);
    inherit_expectation_text(&mut config.diff_from, &inherited.diff_from);
    inherit_expectation_text(&mut config.target, &inherited.target);
}

fn inherit_expectation_text(value: &mut Option<String>, inherited: &Option<String>) {
    if value.is_none() {
        *value = inherited.clone();
    }
}
