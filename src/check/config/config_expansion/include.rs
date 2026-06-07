use crate::config_types::{CooldownConfig, RawExpectationItem, RawExpectationSettings};

pub(super) fn inherit_include_fields(
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
