//! Codex runtime configuration for isolated evaluator processes and threads.

mod context_isolation;
mod filesystem;
mod host_isolation;
mod identity;
mod model;
mod plugins;
mod snapshot;

use super::permissions::FILESYSTEM_READ;
use super::{EvaluatorConfigResult, EVALUATOR_PERMISSION_PROFILE};
use crate::config_types::AgentConfig;
use context_isolation::EvaluatorContextIsolation;
use filesystem::evaluator_filesystem_config_entries;
use model::{
    EvaluatorHistoryConfig, EvaluatorNetworkConfig, EvaluatorPermissionProfile,
    EvaluatorRuntimeConfig,
};
use plugins::{enabled_plugins_config, EnabledPluginConfig};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) use host_isolation::EvaluatorHostIsolation;
pub(crate) use identity::{
    evaluator_thread_config_identity, EvaluatorThreadConfigIdentity,
    EvaluatorThreadConfigIdentityContext,
};
pub(crate) use snapshot::{EvaluatorRuntimeConfigContext, EvaluatorRuntimeConfigSnapshot};

/// Selects who enforces process isolation around the evaluator app-server.
/// Tool capabilities are selected independently and remain read-only in both modes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum EvaluatorProcessIsolation {
    CanonManaged,
    ExternallyManaged,
}

/// Opaque selector serialized only into one ephemeral evaluator thread request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct EphemeralEvaluatorThreadPermissionProfile(&'static str);

impl EvaluatorProcessIsolation {
    /// Selects a predeclared profile for one ephemeral evaluator thread request.
    /// The opaque value is passed only in memory; it is not retained configuration.
    pub(crate) fn ephemeral_thread_permission_profile(
        self,
    ) -> Option<EphemeralEvaluatorThreadPermissionProfile> {
        match self {
            EvaluatorProcessIsolation::CanonManaged => Some(
                EphemeralEvaluatorThreadPermissionProfile(EVALUATOR_PERMISSION_PROFILE),
            ),
            EvaluatorProcessIsolation::ExternallyManaged => None,
        }
    }
}

struct EvaluatorRuntimeSettings<'a> {
    root_access: &'a str,
    reasoning_effort: Option<&'a str>,
    extra_filesystem_permissions: BTreeMap<String, String>,
    process_isolation: EvaluatorProcessIsolation,
    model: Option<String>,
    plugins: Option<BTreeMap<String, EnabledPluginConfig>>,
    codex_executable: &'a Path,
}

pub(super) fn effective_evaluator_thread_model<'a>(
    agent: &'a AgentConfig,
    model: Option<&'a str>,
) -> Option<&'a str> {
    model.or_else(|| agent.models.first().map(String::as_str))
}

pub(crate) fn evaluator_reasoning_effort(thinking: &str) -> Option<&str> {
    Some(thinking)
}

pub(super) fn push_evaluator_startup_config_args(
    args: &mut Vec<String>,
    agent: &AgentConfig,
    process_isolation: EvaluatorProcessIsolation,
    codex_executable: &Path,
) -> EvaluatorConfigResult<()> {
    evaluator_startup_config_settings(agent, codex_executable)
        .with_process_isolation(process_isolation)
        .push_toml_args(args)
}

fn evaluator_startup_config_settings<'a>(
    agent: &'a AgentConfig,
    codex_executable: &'a Path,
) -> EvaluatorRuntimeSettings<'a> {
    EvaluatorRuntimeSettings::new(
        FILESYSTEM_READ,
        evaluator_reasoning_effort(&agent.thinking),
        codex_executable,
    )
    .with_plugins(agent.plugins.iter().map(String::as_str))
}

impl<'a> EvaluatorRuntimeSettings<'a> {
    fn new(
        root_access: &'a str,
        reasoning_effort: Option<&'a str>,
        codex_executable: &'a Path,
    ) -> EvaluatorRuntimeSettings<'a> {
        EvaluatorRuntimeSettings {
            root_access,
            reasoning_effort,
            extra_filesystem_permissions: BTreeMap::new(),
            process_isolation: EvaluatorProcessIsolation::CanonManaged,
            model: None,
            plugins: None,
            codex_executable,
        }
    }

    fn with_extra_filesystem_permissions(
        mut self,
        permissions: BTreeMap<String, String>,
    ) -> EvaluatorRuntimeSettings<'a> {
        self.extra_filesystem_permissions = permissions;
        self
    }

    fn with_process_isolation(
        mut self,
        process_isolation: EvaluatorProcessIsolation,
    ) -> EvaluatorRuntimeSettings<'a> {
        self.process_isolation = process_isolation;
        self
    }

    fn with_model(mut self, model: Option<&str>) -> EvaluatorRuntimeSettings<'a> {
        self.model = model.map(str::to_string);
        self
    }

    fn with_plugins<'b>(
        mut self,
        plugins: impl IntoIterator<Item = &'b str>,
    ) -> EvaluatorRuntimeSettings<'a> {
        let plugins = enabled_plugins_config(plugins);
        if !plugins.is_empty() {
            self.plugins = Some(plugins);
        }
        self
    }

    fn to_config(&self) -> EvaluatorConfigResult<EvaluatorRuntimeConfig> {
        let (sandbox_mode, default_permissions, permissions) = match self.process_isolation {
            EvaluatorProcessIsolation::CanonManaged => {
                let permissions = BTreeMap::from([(
                    EVALUATOR_PERMISSION_PROFILE.to_string(),
                    EvaluatorPermissionProfile {
                        extends: ":read-only",
                        filesystem: evaluator_filesystem_config_entries(
                            self.root_access,
                            &self.extra_filesystem_permissions,
                            self.codex_executable,
                        )?,
                        network: EvaluatorNetworkConfig { enabled: false },
                    },
                )]);
                (None, Some(EVALUATOR_PERMISSION_PROFILE), permissions)
            }
            EvaluatorProcessIsolation::ExternallyManaged => {
                // [l,hQ] This disables only the redundant app-server sandbox
                // because the caller provides process isolation. App-server
                // startup still disables its shell unconditionally, and the
                // ReadOnlyProjectInspectionPlan remains Canon's only built-in
                // local project capability in either process-isolation mode.
                (Some("danger-full-access"), None, BTreeMap::new())
            }
        };
        Ok(EvaluatorRuntimeConfig {
            sandbox_mode,
            model: self.model.clone(),
            default_permissions,
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

#[cfg(test)]
mod tests;
