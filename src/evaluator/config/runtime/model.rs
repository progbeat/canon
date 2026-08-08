//! Serializable Codex runtime configuration model.

use super::super::codec::{
    config_entries_to_json, insert_json_config_override, push_toml_arg, toml_key_segment,
    toml_string, ConfigEntry,
};
use super::super::{EvaluatorConfigError, EvaluatorConfigResult};
use super::context_isolation::EvaluatorContextIsolation;
use super::plugins::EnabledPluginConfig;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub(super) struct EvaluatorRuntimeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sandbox_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) default_permissions: Option<&'static str>,
    pub(super) permissions: BTreeMap<String, EvaluatorPermissionProfile>,
    pub(super) history: EvaluatorHistoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model_reasoning_effort: Option<String>,
    #[serde(flatten)]
    pub(super) context_isolation: EvaluatorContextIsolation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) plugins: Option<BTreeMap<String, EnabledPluginConfig>>,
}

#[derive(Serialize)]
pub(super) struct EvaluatorPermissionProfile {
    pub(super) extends: &'static str,
    pub(super) filesystem: BTreeMap<String, String>,
    pub(super) network: EvaluatorNetworkConfig,
}

#[derive(Serialize)]
pub(super) struct EvaluatorNetworkConfig {
    pub(super) enabled: bool,
}

#[derive(Serialize)]
pub(super) struct EvaluatorHistoryConfig {
    pub(super) persistence: &'static str,
}

impl EvaluatorRuntimeConfig {
    fn config_entries(&self) -> Vec<ConfigEntry> {
        let mut entries = Vec::new();
        if let Some(sandbox_mode) = self.sandbox_mode {
            entries.push(ConfigEntry::string(["sandbox_mode"], sandbox_mode));
        }
        if let Some(model) = &self.model {
            entries.push(ConfigEntry::string(["model"], model));
        }
        if let Some(default_permissions) = self.default_permissions {
            entries.push(ConfigEntry::string(
                ["default_permissions"],
                default_permissions,
            ));
        }
        entries.push(ConfigEntry::string(
            ["history", "persistence"],
            self.history.persistence,
        ));
        if let Some(reasoning_effort) = &self.model_reasoning_effort {
            entries.push(ConfigEntry::string(
                ["model_reasoning_effort"],
                reasoning_effort,
            ));
        }
        self.context_isolation.push_config_entries(&mut entries);
        if let Some(plugins) = &self.plugins {
            for (plugin, config) in plugins {
                config
                    .push_config_entries(&mut entries, vec!["plugins".to_string(), plugin.clone()]);
            }
        }
        entries
    }

    pub(super) fn to_json_value(&self) -> EvaluatorConfigResult<Value> {
        let mut config = config_entries_to_json(self.config_entries())?;
        let root = config
            .as_object_mut()
            .expect("config entry encoding always produces a JSON object");
        for (name, profile) in &self.permissions {
            let profile =
                serde_json::to_value(profile).map_err(|err| EvaluatorConfigError::JsonEncode {
                    context: "evaluator permission profile",
                    message: err.to_string(),
                })?;
            insert_json_config_override(root, &["permissions".to_string(), name.clone()], profile)?;
        }
        Ok(config)
    }

    pub(super) fn push_toml_args(&self, args: &mut Vec<String>) -> EvaluatorConfigResult<()> {
        for entry in self.config_entries() {
            push_toml_arg(args, entry.path, entry.value.to_toml_value());
        }
        for (name, profile) in &self.permissions {
            push_toml_arg(
                args,
                vec!["permissions".to_string(), name.clone()],
                profile.to_toml_inline_table(),
            );
        }
        Ok(())
    }
}

impl EvaluatorPermissionProfile {
    fn to_toml_inline_table(&self) -> String {
        let filesystem = self
            .filesystem
            .iter()
            .map(|(path, value)| format!("{} = {}", toml_key_segment(path), toml_string(value)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{ extends = {}, filesystem = {{ {filesystem} }}, network = {{ enabled = {} }} }}",
            toml_string(self.extends),
            self.network.enabled
        )
    }
}
