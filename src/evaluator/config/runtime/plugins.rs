//! Plugin fields in the Codex runtime configuration.

use super::super::codec::ConfigEntry;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Serialize)]
pub(super) struct EnabledPluginConfig {
    enabled: bool,
}

impl EnabledPluginConfig {
    pub(super) fn push_config_entries(
        &self,
        entries: &mut Vec<ConfigEntry>,
        mut prefix: Vec<String>,
    ) {
        prefix.push("enabled".to_string());
        entries.push(ConfigEntry::bool_path(prefix, self.enabled));
    }
}

pub(super) fn enabled_plugins_config<'a>(
    plugins: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, EnabledPluginConfig> {
    plugins
        .into_iter()
        .map(|plugin| (plugin.to_string(), EnabledPluginConfig { enabled: true }))
        .collect()
}
