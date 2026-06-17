use super::codec::{config_entries_to_json, push_toml_arg, ConfigEntry, ConfigEntryValue};
use super::permissions::{
    evaluator_runtime_permissions, evaluator_state_dir_permissions,
    evaluator_template_output_permissions, evaluator_working_tree_permissions,
    merge_filesystem_permissions, EVALUATOR_FILESYSTEM_GLOB_SCAN_MAX_DEPTH, FILESYSTEM_DENY,
};
use super::{
    EvaluatorConfigResult, EVALUATOR_DISABLED_FEATURES, EVALUATOR_EXTRA_DISABLED_FEATURES,
};
use crate::check::codex_reasoning_effort;
use crate::config_types::AgentConfig;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const EVALUATOR_PERMISSION_PROFILE: &str = "canon_check";

struct EvaluatorConfigSettings<'a> {
    root_access: &'a str,
    reasoning_effort: Option<&'a str>,
    extra_filesystem_permissions: BTreeMap<String, String>,
    sandbox_mode: Option<&'static str>,
    model: Option<String>,
    plugins: Option<BTreeMap<String, EnabledPluginConfig>>,
}

#[derive(Serialize)]
struct EvaluatorBaseConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    default_permissions: &'static str,
    permissions: BTreeMap<String, EvaluatorPermissionProfile>,
    history: EvaluatorHistoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_reasoning_effort: Option<String>,
    #[serde(flatten)]
    context_isolation: EvaluatorContextIsolation,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugins: Option<BTreeMap<String, EnabledPluginConfig>>,
}

#[derive(Serialize)]
struct EvaluatorPermissionProfile {
    filesystem: BTreeMap<String, FilesystemConfigValue>,
    network: EvaluatorNetworkConfig,
}

#[derive(Serialize)]
struct EvaluatorNetworkConfig {
    enabled: bool,
}

#[derive(Serialize)]
struct EvaluatorHistoryConfig {
    persistence: &'static str,
}

#[derive(Serialize)]
struct EvaluatorContextIsolation {
    include_environment_context: bool,
    include_permissions_instructions: bool,
    include_apps_instructions: bool,
    include_apply_patch_tool: bool,
    experimental_use_freeform_apply_patch: bool,
    features: BTreeMap<String, bool>,
    project_doc_max_bytes: u64,
}

#[derive(Clone, Serialize)]
struct EnabledPluginConfig {
    enabled: bool,
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
enum FilesystemConfigValue {
    String(String),
    U64(u64),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluator_thread_config_with_no_sandbox(
    agent: &AgentConfig,
    _scope: &[String],
    model: Option<&str>,
    thinking: &str,
    app_server_root: &Path,
    session_root: &Path,
    template_output_dir: &Path,
    no_sandbox: bool,
) -> EvaluatorConfigResult<Value> {
    // Scope and ignore filtering is enforced by the materialized evaluator
    // working tree. App-server permissions only sandbox that already-filtered
    // cwd, so they must not encode scoped project paths.
    let mut extra_permissions = BTreeMap::new();
    merge_filesystem_permissions(
        &mut extra_permissions,
        evaluator_working_tree_permissions(session_root)?,
    )?;
    merge_filesystem_permissions(
        &mut extra_permissions,
        evaluator_state_dir_permissions(app_server_root)?,
    )?;
    merge_filesystem_permissions(
        &mut extra_permissions,
        evaluator_template_output_permissions(template_output_dir)?,
    )?;
    EvaluatorConfigSettings::new(FILESYSTEM_DENY, codex_reasoning_effort(thinking))
        .with_extra_filesystem_permissions(extra_permissions)
        .with_sandbox_mode(no_sandbox.then_some("danger-full-access"))
        .with_model(model.or_else(|| agent.models.first().map(String::as_str)))
        .with_plugins(agent)
        .to_json_value()
}

pub(super) fn push_evaluator_startup_config_args(
    args: &mut Vec<String>,
    agent: &AgentConfig,
) -> EvaluatorConfigResult<()> {
    EvaluatorConfigSettings::new("read", codex_reasoning_effort(&agent.thinking))
        .push_toml_args(args)
}

fn evaluator_filesystem_config_entries(
    root_access: &str,
    extra_permissions: &BTreeMap<String, String>,
) -> EvaluatorConfigResult<BTreeMap<String, FilesystemConfigValue>> {
    let mut entries = BTreeMap::new();
    insert_filesystem_config_entry(
        &mut entries,
        ":root".to_string(),
        FilesystemConfigValue::String(root_access.to_string()),
    )?;
    insert_filesystem_config_entry(
        &mut entries,
        ":minimal".to_string(),
        FilesystemConfigValue::String("read".to_string()),
    )?;
    for (path, permission) in evaluator_runtime_permissions()? {
        insert_filesystem_config_entry(
            &mut entries,
            path,
            FilesystemConfigValue::String(permission),
        )?;
    }
    for (path, permission) in extra_permissions {
        insert_filesystem_config_entry(
            &mut entries,
            path.clone(),
            FilesystemConfigValue::String(permission.clone()),
        )?;
    }
    insert_filesystem_config_entry(
        &mut entries,
        "glob_scan_max_depth".to_string(),
        FilesystemConfigValue::U64(EVALUATOR_FILESYSTEM_GLOB_SCAN_MAX_DEPTH),
    )?;
    Ok(entries)
}

fn insert_filesystem_config_entry(
    entries: &mut BTreeMap<String, FilesystemConfigValue>,
    path: String,
    value: FilesystemConfigValue,
) -> EvaluatorConfigResult<()> {
    if entries.contains_key(&path) {
        return Err(super::EvaluatorConfigError::DuplicateFilesystemConfigEntry { path });
    }
    entries.insert(path, value);
    Ok(())
}

impl<'a> EvaluatorConfigSettings<'a> {
    fn new(root_access: &'a str, reasoning_effort: Option<&'a str>) -> EvaluatorConfigSettings<'a> {
        EvaluatorConfigSettings {
            root_access,
            reasoning_effort,
            extra_filesystem_permissions: BTreeMap::new(),
            sandbox_mode: None,
            model: None,
            plugins: None,
        }
    }

    fn with_extra_filesystem_permissions(
        mut self,
        permissions: BTreeMap<String, String>,
    ) -> EvaluatorConfigSettings<'a> {
        self.extra_filesystem_permissions = permissions;
        self
    }

    fn with_sandbox_mode(
        mut self,
        sandbox_mode: Option<&'static str>,
    ) -> EvaluatorConfigSettings<'a> {
        self.sandbox_mode = sandbox_mode;
        self
    }

    fn with_model(mut self, model: Option<&str>) -> EvaluatorConfigSettings<'a> {
        self.model = model.map(str::to_string);
        self
    }

    fn with_plugins(mut self, agent: &AgentConfig) -> EvaluatorConfigSettings<'a> {
        if !agent.plugins.is_empty() {
            self.plugins = Some(enabled_plugins_config(agent));
        }
        self
    }

    fn to_config(&self) -> EvaluatorConfigResult<EvaluatorBaseConfig> {
        let mut permissions = BTreeMap::new();
        permissions.insert(
            EVALUATOR_PERMISSION_PROFILE.to_string(),
            EvaluatorPermissionProfile {
                filesystem: evaluator_filesystem_config_entries(
                    self.root_access,
                    &self.extra_filesystem_permissions,
                )?,
                network: EvaluatorNetworkConfig { enabled: false },
            },
        );
        Ok(EvaluatorBaseConfig {
            sandbox_mode: self.sandbox_mode,
            model: self.model.clone(),
            default_permissions: EVALUATOR_PERMISSION_PROFILE,
            permissions,
            history: EvaluatorHistoryConfig {
                persistence: "none",
            },
            model_reasoning_effort: self.reasoning_effort.map(str::to_string),
            context_isolation: EvaluatorContextIsolation::disabled(),
            plugins: self.plugins.clone(),
        })
    }

    fn to_json_value(&self) -> EvaluatorConfigResult<Value> {
        self.to_config()?.to_json_value()
    }

    fn push_toml_args(&self, args: &mut Vec<String>) -> EvaluatorConfigResult<()> {
        self.to_config()?.push_toml_args(args)
    }
}

impl EvaluatorBaseConfig {
    fn config_entries(&self) -> Vec<ConfigEntry> {
        let mut entries = Vec::new();
        if let Some(sandbox_mode) = self.sandbox_mode {
            entries.push(ConfigEntry::string(["sandbox_mode"], sandbox_mode));
        }
        if let Some(model) = &self.model {
            entries.push(ConfigEntry::string(["model"], model));
        }
        entries.push(ConfigEntry::string(
            ["default_permissions"],
            self.default_permissions,
        ));
        for (name, profile) in &self.permissions {
            profile
                .push_config_entries(&mut entries, vec!["permissions".to_string(), name.clone()]);
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

    fn to_json_value(&self) -> EvaluatorConfigResult<Value> {
        config_entries_to_json(self.config_entries())
    }

    fn push_toml_args(&self, args: &mut Vec<String>) -> EvaluatorConfigResult<()> {
        for entry in self.config_entries() {
            push_toml_arg(args, entry.path, entry.value.to_toml_value());
        }
        Ok(())
    }
}

impl EvaluatorPermissionProfile {
    fn push_config_entries(&self, entries: &mut Vec<ConfigEntry>, prefix: Vec<String>) {
        for (path, value) in &self.filesystem {
            let mut key = prefix.clone();
            key.push("filesystem".to_string());
            key.push(path.clone());
            entries.push(ConfigEntry {
                path: key,
                value: value.to_config_entry_value(),
            });
        }
        let mut network_prefix = prefix;
        network_prefix.push("network".to_string());
        self.network.push_config_entries(entries, network_prefix);
    }
}

impl EvaluatorNetworkConfig {
    fn push_config_entries(&self, entries: &mut Vec<ConfigEntry>, mut prefix: Vec<String>) {
        prefix.push("enabled".to_string());
        entries.push(ConfigEntry::bool_path(prefix, self.enabled));
    }
}

impl EnabledPluginConfig {
    fn push_config_entries(&self, entries: &mut Vec<ConfigEntry>, mut prefix: Vec<String>) {
        prefix.push("enabled".to_string());
        entries.push(ConfigEntry::bool_path(prefix, self.enabled));
    }
}

impl FilesystemConfigValue {
    fn to_config_entry_value(&self) -> ConfigEntryValue {
        match self {
            FilesystemConfigValue::String(value) => ConfigEntryValue::String(value.clone()),
            FilesystemConfigValue::U64(value) => ConfigEntryValue::U64(*value),
        }
    }
}

impl EvaluatorContextIsolation {
    fn disabled() -> EvaluatorContextIsolation {
        EvaluatorContextIsolation {
            include_environment_context: false,
            include_permissions_instructions: false,
            include_apps_instructions: false,
            include_apply_patch_tool: false,
            experimental_use_freeform_apply_patch: false,
            features: evaluator_context_isolation_features()
                .map(|feature| (feature.to_string(), false))
                .collect(),
            project_doc_max_bytes: 0,
        }
    }

    fn push_config_entries(&self, entries: &mut Vec<ConfigEntry>) {
        entries.push(ConfigEntry::bool(
            ["include_environment_context"],
            self.include_environment_context,
        ));
        entries.push(ConfigEntry::bool(
            ["include_permissions_instructions"],
            self.include_permissions_instructions,
        ));
        entries.push(ConfigEntry::bool(
            ["include_apps_instructions"],
            self.include_apps_instructions,
        ));
        entries.push(ConfigEntry::bool(
            ["include_apply_patch_tool"],
            self.include_apply_patch_tool,
        ));
        entries.push(ConfigEntry::bool(
            ["experimental_use_freeform_apply_patch"],
            self.experimental_use_freeform_apply_patch,
        ));
        for (feature, enabled) in &self.features {
            entries.push(ConfigEntry::bool_path(
                vec!["features".to_string(), feature.clone()],
                *enabled,
            ));
        }
        entries.push(ConfigEntry::u64(
            ["project_doc_max_bytes"],
            self.project_doc_max_bytes,
        ));
    }
}

fn evaluator_context_isolation_features() -> impl Iterator<Item = &'static str> {
    EVALUATOR_DISABLED_FEATURES
        .iter()
        .copied()
        .chain(EVALUATOR_EXTRA_DISABLED_FEATURES.iter().copied())
}

fn enabled_plugins_config(agent: &AgentConfig) -> BTreeMap<String, EnabledPluginConfig> {
    let mut plugins = BTreeMap::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), EnabledPluginConfig { enabled: true });
    }
    plugins
}
