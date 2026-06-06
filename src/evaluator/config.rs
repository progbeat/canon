use crate::check::codex_reasoning_effort;
use crate::config_types::AgentConfig;
use crate::fs_util::write_temp_file_then_replace;
use crate::git::resolve_git_path;
use crate::logs::{thread_reuse_config, ThreadReuseConfig};
use crate::platform;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const EVALUATOR_MODEL_CATALOG_TEMP_DIR: &str = "canon-evaluator-model-catalogs";
const FILESYSTEM_DENY: &str = "deny";
const EVALUATOR_PERMISSION_PROFILE: &str = "canon_check";
const EVALUATOR_FILESYSTEM_GLOB_SCAN_MAX_DEPTH: u64 = 32;
static MODEL_CATALOG_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
// Evaluators should see only Canon's own instructions plus the essential
// shell-command read/exec path governed by the read-only permission profile.
const EVALUATOR_DISABLED_FEATURES: &[&str] = &[
    "apps",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "fast_mode",
    "guardian_approval",
    "hooks",
    "image_generation",
    "in_app_browser",
    "multi_agent",
    "personality",
    "plugin_hooks",
    "shell_snapshot",
    "skill_mcp_dependency_install",
    "terminal_resize_reflow",
    "tool_call_mcp_elicitation",
    "tool_search",
    "tool_suggest",
    "unavailable_dummy_tools",
    "unified_exec",
    "workspace_dependencies",
];
const EVALUATOR_EXTRA_DISABLED_FEATURES: &[&str] = &["apply_patch_freeform"];

type EvaluatorConfigResult<T> = Result<T, EvaluatorConfigError>;

#[derive(Debug)]
pub(crate) enum EvaluatorConfigError {
    Message(String),
    DuplicateConfigEntry {
        path: String,
    },
    DuplicateFilesystemConfigEntry {
        path: String,
    },
    DuplicateFilesystemPermission {
        path: String,
        existing: String,
        replacement: String,
    },
    HomeNotUtf8,
    InvalidPathUtf8 {
        context: &'static str,
    },
    JsonEncode {
        context: &'static str,
        message: String,
    },
}

pub(crate) struct AppServerArgs {
    pub(crate) args: Vec<String>,
    pub(crate) model_catalog_file: Option<ModelCatalogFile>,
}

pub(crate) struct ModelCatalogFile {
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum AppServerModelKey {
    Default,
    Named(String),
}

struct StartupConfigArgs {
    args: Vec<String>,
    model_catalog_file: Option<ModelCatalogFile>,
}

struct ModelCatalogConfigArg {
    arg: String,
    file: ModelCatalogFile,
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
enum FilesystemConfigValue {
    String(String),
    U64(u64),
}

struct ConfigEntry {
    path: Vec<String>,
    value: ConfigEntryValue,
}

enum ConfigEntryValue {
    String(String),
    Bool(bool),
    U64(u64),
}

impl fmt::Display for EvaluatorConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvaluatorConfigError::Message(message) => formatter.write_str(message),
            EvaluatorConfigError::DuplicateConfigEntry { path } => {
                write!(formatter, "duplicate evaluator config entry for {}", path)
            }
            EvaluatorConfigError::DuplicateFilesystemConfigEntry { path } => {
                write!(
                    formatter,
                    "duplicate evaluator filesystem config entry for {}",
                    path
                )
            }
            EvaluatorConfigError::DuplicateFilesystemPermission {
                path,
                existing,
                replacement,
            } => write!(
                formatter,
                "duplicate evaluator filesystem permission for {}: {} and {}",
                path, existing, replacement
            ),
            EvaluatorConfigError::HomeNotUtf8 => {
                formatter.write_str("HOME must be valid UTF-8 for evaluator runtime permissions")
            }
            EvaluatorConfigError::InvalidPathUtf8 { context } => {
                write!(formatter, "{} must be valid UTF-8", context)
            }
            EvaluatorConfigError::JsonEncode { context, message } => {
                write!(formatter, "failed to encode {}: {}", context, message)
            }
        }
    }
}

impl std::error::Error for EvaluatorConfigError {}

impl From<String> for EvaluatorConfigError {
    fn from(message: String) -> EvaluatorConfigError {
        EvaluatorConfigError::Message(message)
    }
}

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

#[derive(Serialize)]
struct EvaluatorModelCatalog<'a> {
    models: Vec<EvaluatorModelCatalogEntry<'a>>,
}

#[derive(Serialize)]
struct EvaluatorModelCatalogEntry<'a> {
    slug: &'a str,
    display_name: &'a str,
    description: &'static str,
    default_reasoning_level: &'static str,
    supported_reasoning_levels: Vec<EvaluatorReasoningLevel>,
    shell_type: &'static str,
    visibility: &'static str,
    supported_in_api: bool,
    priority: u64,
    base_instructions: &'static str,
    supports_reasoning_summaries: bool,
    default_reasoning_summary: &'static str,
    support_verbosity: bool,
    default_verbosity: &'static str,
    apply_patch_tool_type: Option<&'static str>,
    truncation_policy: EvaluatorTruncationPolicy,
    supports_parallel_tool_calls: bool,
    supports_image_detail_original: bool,
    context_window: u64,
    max_context_window: u64,
    effective_context_window_percent: u64,
    experimental_supported_tools: Vec<&'static str>,
    input_modalities: Vec<&'static str>,
    supports_search_tool: bool,
}

#[derive(Serialize)]
struct EvaluatorReasoningLevel {
    effort: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct EvaluatorTruncationPolicy {
    mode: &'static str,
    limit: u64,
}

pub(crate) fn evaluator_thread_config_with_no_sandbox(
    agent: &AgentConfig,
    _scope: &[String],
    model: Option<&str>,
    thinking: &str,
    app_server_root: &Path,
    session_root: &Path,
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
    EvaluatorConfigSettings::new(FILESYSTEM_DENY, codex_reasoning_effort(thinking))
        .with_extra_filesystem_permissions(extra_permissions)
        .with_sandbox_mode(no_sandbox.then_some("danger-full-access"))
        .with_model(model.or_else(|| agent.models.first().map(String::as_str)))
        .with_plugins(agent)
        .to_json_value()
}

pub(crate) fn evaluator_working_tree_permissions(
    session_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    insert_filesystem_permission(
        &mut permissions,
        absolute_session_path(session_root, ".")?,
        "read",
    )?;
    insert_filesystem_permission(
        &mut permissions,
        absolute_session_glob(session_root, "**")?,
        "read",
    )?;
    Ok(permissions)
}

pub(crate) fn evaluator_state_dir_permissions(
    app_server_root: &Path,
) -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    let state_root = resolve_git_path(
        app_server_root,
        crate::state_paths::CANON_STATE_DIR_GIT_PATH,
    )?;
    insert_tree_permission(&mut permissions, &state_root, FILESYSTEM_DENY)?;
    Ok(permissions)
}

fn insert_tree_permission(
    permissions: &mut BTreeMap<String, String>,
    path: &Path,
    permission: &str,
) -> EvaluatorConfigResult<()> {
    let path = path_to_config_string(path, "evaluator filesystem permission path")?;
    let path = path.trim_end_matches('/').to_string();
    insert_filesystem_permission(permissions, path.clone(), permission)?;
    insert_filesystem_permission(permissions, format!("{}/**", path), permission)?;
    Ok(())
}

fn merge_filesystem_permissions(
    target: &mut BTreeMap<String, String>,
    source: BTreeMap<String, String>,
) -> EvaluatorConfigResult<()> {
    for (path, permission) in source {
        insert_filesystem_permission(target, path, &permission)?;
    }
    Ok(())
}

fn insert_filesystem_permission(
    permissions: &mut BTreeMap<String, String>,
    path: String,
    permission: &str,
) -> EvaluatorConfigResult<()> {
    if let Some(existing) = permissions.get(&path) {
        return Err(EvaluatorConfigError::DuplicateFilesystemPermission {
            path,
            existing: existing.clone(),
            replacement: permission.to_string(),
        });
    }
    permissions.insert(path, permission.to_string());
    Ok(())
}

fn absolute_session_path(session_root: &Path, path: &str) -> EvaluatorConfigResult<String> {
    let path = if path == "." {
        session_root.to_path_buf()
    } else {
        session_root.join(path)
    };
    path_to_config_string(&path, "evaluator session path")
}

fn absolute_session_glob(session_root: &Path, pattern: &str) -> EvaluatorConfigResult<String> {
    path_to_config_string(&session_root.join(pattern), "evaluator session glob path")
}

fn path_to_config_string(path: &Path, context: &'static str) -> EvaluatorConfigResult<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or(EvaluatorConfigError::InvalidPathUtf8 { context })
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
        return Err(EvaluatorConfigError::DuplicateFilesystemConfigEntry { path });
    }
    entries.insert(path, value);
    Ok(())
}

impl ConfigEntry {
    fn string<const N: usize>(path: [&str; N], value: &str) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::String(value.to_string()),
        }
    }

    fn bool<const N: usize>(path: [&str; N], value: bool) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::Bool(value),
        }
    }

    fn bool_path(path: Vec<String>, value: bool) -> ConfigEntry {
        ConfigEntry {
            path,
            value: ConfigEntryValue::Bool(value),
        }
    }

    fn u64<const N: usize>(path: [&str; N], value: u64) -> ConfigEntry {
        ConfigEntry {
            path: path.iter().map(|part| (*part).to_string()).collect(),
            value: ConfigEntryValue::U64(value),
        }
    }
}

impl ConfigEntryValue {
    fn to_json_value(&self) -> Value {
        match self {
            ConfigEntryValue::String(value) => Value::String(value.clone()),
            ConfigEntryValue::Bool(value) => Value::Bool(*value),
            ConfigEntryValue::U64(value) => Value::Number((*value).into()),
        }
    }

    fn to_toml_value(&self) -> String {
        match self {
            ConfigEntryValue::String(value) => toml_string(value),
            ConfigEntryValue::Bool(value) => value.to_string(),
            ConfigEntryValue::U64(value) => value.to_string(),
        }
    }
}

fn config_entries_to_json(entries: Vec<ConfigEntry>) -> EvaluatorConfigResult<Value> {
    let mut root = Value::Object(serde_json::Map::new());
    for entry in entries {
        insert_json_config_entry(&mut root, &entry.path, entry.value)?;
    }
    Ok(root)
}

fn insert_json_config_entry(
    root: &mut Value,
    path: &[String],
    value: ConfigEntryValue,
) -> EvaluatorConfigResult<()> {
    let Some((last, parents)) = path.split_last() else {
        return Err(EvaluatorConfigError::Message(
            "evaluator config entry path must not be empty".to_string(),
        ));
    };
    let mut cursor = root;
    for part in parents {
        let object = cursor
            .as_object_mut()
            .ok_or_else(|| format!("evaluator config path conflicts before {}", part))?;
        cursor = object
            .entry(part.clone())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    let object = cursor
        .as_object_mut()
        .ok_or_else(|| format!("evaluator config path conflicts at {}", last))?;
    if object.contains_key(last) {
        return Err(EvaluatorConfigError::DuplicateConfigEntry {
            path: path.join("."),
        });
    }
    object.insert(last.clone(), value.to_json_value());
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

pub(crate) fn evaluator_runtime_permissions() -> EvaluatorConfigResult<BTreeMap<String, String>> {
    let mut permissions = BTreeMap::new();
    for path in [
        "~",
        "~/.zlogin",
        "~/.zlogout",
        "~/.zprofile",
        "~/.zshenv",
        "~/.zshrc",
        "/etc/**",
        "/private/etc/**",
        "/bin/**",
        "/usr/bin/**",
        "/usr/lib/**",
        "/usr/libexec/**",
        "/usr/share/**",
        "/System/**",
        "/Library/**",
        "/opt/homebrew/**",
    ] {
        insert_filesystem_permission(&mut permissions, path.to_string(), "read")?;
    }
    deny_runtime_path(&mut permissions, ":tmpdir")?;
    deny_runtime_path(&mut permissions, ":slash_tmp")?;
    deny_runtime_path(&mut permissions, "/dev/null")?;
    deny_runtime_tree(&mut permissions, "/tmp")?;
    deny_runtime_tree(&mut permissions, "/private/tmp")?;
    deny_runtime_tree(&mut permissions, "~/.codex/sessions")?;
    deny_runtime_tree(&mut permissions, "~/.codex/memories")?;
    add_home_runtime_permissions(&mut permissions, env::var_os("HOME"))?;
    Ok(permissions)
}

fn add_home_runtime_permissions(
    permissions: &mut BTreeMap<String, String>,
    home: Option<OsString>,
) -> EvaluatorConfigResult<()> {
    if let Some(home) = home {
        let home = home
            .into_string()
            .map_err(|_| EvaluatorConfigError::HomeNotUtf8)?;
        let codex_home = format!("{}/.codex", home.trim_end_matches('/'));
        deny_runtime_tree(permissions, &format!("{}/sessions", codex_home))?;
        deny_runtime_tree(permissions, &format!("{}/memories", codex_home))?;
    }
    Ok(())
}

fn deny_runtime_path(
    permissions: &mut BTreeMap<String, String>,
    path: &str,
) -> EvaluatorConfigResult<()> {
    insert_filesystem_permission(permissions, path.to_string(), FILESYSTEM_DENY)
}

fn deny_runtime_tree(
    permissions: &mut BTreeMap<String, String>,
    path: &str,
) -> EvaluatorConfigResult<()> {
    deny_runtime_path(permissions, path)?;
    deny_runtime_path(permissions, &format!("{}/**", path))
}

fn enabled_plugins_config(agent: &AgentConfig) -> BTreeMap<String, EnabledPluginConfig> {
    let mut plugins = BTreeMap::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), EnabledPluginConfig { enabled: true });
    }
    plugins
}

pub(crate) fn app_server_args_with_no_sandbox(
    root: &Path,
    load_plugins: bool,
    agent: &AgentConfig,
    no_sandbox: bool,
) -> EvaluatorConfigResult<AppServerArgs> {
    let mut args = vec!["app-server".to_string()];
    for feature in evaluator_disabled_app_server_features(load_plugins) {
        args.push("--disable".to_string());
        args.push(feature.to_string());
    }
    let startup_config = app_server_startup_config_args_with_no_sandbox(root, agent, no_sandbox)?;
    args.extend(startup_config.args);
    args.push("--listen".to_string());
    args.push("stdio://".to_string());
    Ok(AppServerArgs {
        args,
        model_catalog_file: startup_config.model_catalog_file,
    })
}

fn evaluator_disabled_app_server_features(load_plugins: bool) -> Vec<&'static str> {
    let mut features = Vec::new();
    if !load_plugins {
        features.push("plugins");
    }
    features.extend(EVALUATOR_DISABLED_FEATURES.iter().copied());
    features.extend(EVALUATOR_EXTRA_DISABLED_FEATURES.iter().copied());
    features
}

fn app_server_startup_config_args_with_no_sandbox(
    root: &Path,
    agent: &AgentConfig,
    no_sandbox: bool,
) -> EvaluatorConfigResult<StartupConfigArgs> {
    let thread_reuse = thread_reuse_config(root)?;
    let mut args = Vec::new();
    let mut model_catalog_file = None;
    if no_sandbox {
        // Docker supplies the outer isolation boundary. Keep Canon's
        // permission profile below so evaluator tools are still confined to
        // the materialized snapshot, while avoiding the host OS sandbox
        // launcher that is unavailable in the container.
        push_config_arg(&mut args, "sandbox_mode=\"danger-full-access\"");
    }
    EvaluatorConfigSettings::new("read", codex_reasoning_effort(&agent.thinking))
        .push_toml_args(&mut args)?;
    if let Some(model_catalog_arg) = evaluator_model_catalog_config_arg(agent)? {
        push_config_arg(&mut args, &model_catalog_arg.arg);
        model_catalog_file = Some(model_catalog_arg.file);
    }
    push_config_arg(
        &mut args,
        &thread_reuse_carryover_token_target_arg(&thread_reuse),
    );
    Ok(StartupConfigArgs {
        args,
        model_catalog_file,
    })
}

fn evaluator_model_catalog_config_arg(
    agent: &AgentConfig,
) -> EvaluatorConfigResult<Option<ModelCatalogConfigArg>> {
    let models = evaluator_model_catalog_slugs(agent);
    if models.is_empty() {
        return Ok(None);
    }
    let file = write_evaluator_model_catalog(&models)?;
    let path_arg = path_to_config_string(file.path(), "evaluator model catalog path")?;
    Ok(Some(ModelCatalogConfigArg {
        arg: format!("model_catalog_json={}", toml_string(&path_arg)),
        file,
    }))
}

fn evaluator_model_catalog_slugs(agent: &AgentConfig) -> Vec<String> {
    let mut models = Vec::new();
    for model in &agent.models {
        push_unique_model_slug(&mut models, model);
    }
    models
}

fn push_unique_model_slug(models: &mut Vec<String>, model: &str) {
    if !models.iter().any(|existing| existing == model) {
        models.push(model.to_string());
    }
}

fn write_evaluator_model_catalog(models: &[String]) -> EvaluatorConfigResult<ModelCatalogFile> {
    let dir = evaluator_model_catalog_dir()?;
    let file_stem = evaluator_model_catalog_file_stem()?;
    let path = dir.join(format!("{}.json", file_stem));
    let temp_path = dir.join(format!("{}.tmp", file_stem));
    let catalog = evaluator_model_catalog_json(models)?;
    write_temp_file_then_replace(&temp_path, &path, |file| {
        file.write_all(catalog.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))
    })?;
    Ok(ModelCatalogFile::new(path))
}

fn evaluator_model_catalog_file_stem() -> EvaluatorConfigResult<String> {
    let sequence = MODEL_CATALOG_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("failed to read system time: {}", err))?
        .as_nanos();
    Ok(format!("{}.{}.{}", std::process::id(), sequence, timestamp))
}

fn evaluator_model_catalog_dir() -> EvaluatorConfigResult<PathBuf> {
    let temp_root = env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))?;
    let dir = temp_root.join(EVALUATOR_MODEL_CATALOG_TEMP_DIR);
    platform::create_private_dir_all(&dir).map_err(|err| {
        format!(
            "failed to create evaluator model catalog dir {}: {}",
            dir.display(),
            err
        )
    })?;
    Ok(dir)
}

pub(crate) fn evaluator_model_catalog_json(models: &[String]) -> EvaluatorConfigResult<String> {
    let catalog = EvaluatorModelCatalog {
        models: models
            .iter()
            .map(|model| evaluator_model_catalog_entry(model))
            .collect(),
    };
    serde_json::to_string(&catalog).map_err(|err| EvaluatorConfigError::JsonEncode {
        context: "evaluator model catalog",
        message: err.to_string(),
    })
}

fn evaluator_model_catalog_entry(model: &str) -> EvaluatorModelCatalogEntry<'_> {
    EvaluatorModelCatalogEntry {
        slug: model,
        display_name: model,
        description: "Canon evaluator model",
        default_reasoning_level: "medium",
        supported_reasoning_levels: vec![
            EvaluatorReasoningLevel {
                effort: "low",
                description: "Low",
            },
            EvaluatorReasoningLevel {
                effort: "medium",
                description: "Medium",
            },
            EvaluatorReasoningLevel {
                effort: "high",
                description: "High",
            },
            EvaluatorReasoningLevel {
                effort: "xhigh",
                description: "Extra high",
            },
        ],
        shell_type: "shell_command",
        visibility: "list",
        supported_in_api: true,
        priority: 0,
        base_instructions: "",
        supports_reasoning_summaries: true,
        default_reasoning_summary: "none",
        support_verbosity: true,
        default_verbosity: "low",
        apply_patch_tool_type: None,
        truncation_policy: EvaluatorTruncationPolicy {
            mode: "tokens",
            limit: 10000,
        },
        supports_parallel_tool_calls: true,
        supports_image_detail_original: true,
        context_window: 272000,
        max_context_window: 1000000,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: vec!["text"],
        supports_search_tool: false,
    }
}

fn evaluator_context_isolation_features() -> impl Iterator<Item = &'static str> {
    EVALUATOR_DISABLED_FEATURES
        .iter()
        .copied()
        .chain(EVALUATOR_EXTRA_DISABLED_FEATURES.iter().copied())
}

pub(crate) fn thread_reuse_carryover_token_target_arg(config: &ThreadReuseConfig) -> String {
    format!(
        "thread_reuse.carryover_token_target=[{},{}]",
        config.carryover_token_target.min, config.carryover_token_target.max
    )
}

pub(crate) fn app_server_model_key(model: Option<&str>) -> AppServerModelKey {
    match model {
        Some(model) => AppServerModelKey::Named(model.to_string()),
        None => AppServerModelKey::Default,
    }
}

impl AppServerModelKey {
    pub(crate) fn push_cache_key_part(&self, key: &mut String) {
        match self {
            AppServerModelKey::Default => key.push_str("default"),
            AppServerModelKey::Named(model) => {
                key.push_str("named");
                key.push('\0');
                key.push_str(&model.len().to_string());
                key.push('\0');
                key.push_str(model);
            }
        }
    }
}

impl ModelCatalogFile {
    fn new(path: PathBuf) -> ModelCatalogFile {
        ModelCatalogFile { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ModelCatalogFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn push_toml_arg(args: &mut Vec<String>, path: Vec<String>, value: String) {
    push_config_arg(args, &format!("{}={}", toml_dotted_key(&path), value));
}

fn toml_dotted_key(path: &[String]) -> String {
    path.iter()
        .map(|segment| toml_key_segment(segment))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn push_config_arg(args: &mut Vec<String>, value: &str) {
    args.push("-c".to_string());
    args.push(value.to_string());
}

pub(crate) fn toml_key_segment(value: &str) -> String {
    toml_string(value)
}

pub(crate) fn toml_string(value: &str) -> String {
    // TOML basic strings use the same delimiters and escape forms needed for
    // the values canon emits here, so the JSON string serializer gives us a
    // battle-tested quoted string. JSON may leave DEL/C1 controls literal, so
    // patch only those TOML-forbidden characters after JSON has handled the
    // common string grammar.
    let mut encoded =
        serde_json::to_string(value).expect("serializing a TOML basic string cannot fail");
    for ch in value.chars().filter(|ch| ch.is_control() && *ch > '\u{1f}') {
        encoded = encoded.replace(ch, &format!("\\u{:04X}", ch as u32));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_permissions_deny_common_temp_entry_points() {
        let permissions = evaluator_runtime_permissions().unwrap();

        for path in [
            ":tmpdir",
            ":slash_tmp",
            "/dev/null",
            "/tmp",
            "/tmp/**",
            "/private/tmp",
            "/private/tmp/**",
        ] {
            assert_permission(&permissions, path, FILESYSTEM_DENY);
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_permissions_reject_non_utf8_home() {
        use std::os::unix::ffi::OsStringExt;

        let mut permissions = BTreeMap::new();
        let home = OsString::from_vec(b"/tmp/canon-home-\xff".to_vec());

        assert!(add_home_runtime_permissions(&mut permissions, Some(home)).is_err());
        assert!(permissions.is_empty());
    }

    #[test]
    fn working_tree_permissions_read_session_root_and_children() {
        let session_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("canon-materialized-tree");
        let permissions = evaluator_working_tree_permissions(&session_root).unwrap();
        let root_key = path_to_config_string(&session_root, "test session root").unwrap();
        let children_key =
            path_to_config_string(&session_root.join("**"), "test session children").unwrap();

        assert_eq!(permissions.get(&root_key), Some(&"read".to_string()));
        assert_eq!(permissions.get(&children_key), Some(&"read".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn working_tree_permissions_reject_non_utf8_session_root() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let session_root = Path::new(OsStr::from_bytes(b"/tmp/canon-\xff"));

        assert!(evaluator_working_tree_permissions(session_root).is_err());
    }

    #[test]
    fn state_dir_permissions_deny_canon_state_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let state_root =
            resolve_git_path(root, crate::state_paths::CANON_STATE_DIR_GIT_PATH).unwrap();
        let state_root = path_to_config_string(&state_root, "test state root").unwrap();
        let permissions = evaluator_state_dir_permissions(root).unwrap();

        assert_eq!(
            permissions.get(&state_root),
            Some(&FILESYSTEM_DENY.to_string())
        );
        assert_eq!(
            permissions.get(&format!("{}/**", state_root)),
            Some(&FILESYSTEM_DENY.to_string())
        );
    }

    #[test]
    fn model_catalog_paths_are_unique_per_write() {
        let models = vec!["gpt-test".to_string()];

        let first = write_evaluator_model_catalog(&models).unwrap();
        let second = write_evaluator_model_catalog(&models).unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();

        assert_ne!(first_path, second_path);
        assert!(first_path.exists());
        assert!(second_path.exists());
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    fn assert_permission(permissions: &BTreeMap<String, String>, path: &str, expected: &str) {
        assert_eq!(permissions.get(path), Some(&expected.to_string()));
    }
}
