//! Hashable values captured for one evaluator runtime configuration.

use super::super::permissions::{
    evaluator_resolved_state_dir_permissions, evaluator_template_artifact_permissions,
    evaluator_working_tree_read_exception, merge_filesystem_permissions, FILESYSTEM_READ,
};
use super::super::EvaluatorConfigResult;
use super::{
    effective_evaluator_thread_model, evaluator_reasoning_effort, EvaluatorHostIsolation,
    EvaluatorProcessIsolation, EvaluatorRuntimeSettings,
};
use crate::config_types::AgentConfig;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) struct EvaluatorRuntimeConfigContext<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) app_server_state_root: Option<&'a Path>,
    pub(crate) session_root: &'a Path,
    pub(crate) template_artifact_directory: &'a Path,
    pub(crate) host_isolation: &'a EvaluatorHostIsolation,
    pub(crate) process_isolation: EvaluatorProcessIsolation,
    pub(crate) codex_executable: &'a Path,
}

/// Opaque evaluator runtime values captured from caller state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EvaluatorRuntimeConfigSnapshot {
    model: Option<String>,
    reasoning_effort: Option<String>,
    plugins: BTreeSet<String>,
    app_server_state_root: Option<PathBuf>,
    session_root: PathBuf,
    template_artifact_directory: PathBuf,
    host_isolation: EvaluatorHostIsolation,
    process_isolation: EvaluatorProcessIsolation,
    codex_executable: PathBuf,
}

impl EvaluatorRuntimeConfigSnapshot {
    pub(crate) fn capture(
        context: EvaluatorRuntimeConfigContext<'_>,
    ) -> EvaluatorRuntimeConfigSnapshot {
        EvaluatorRuntimeConfigSnapshot {
            model: effective_evaluator_thread_model(context.agent, context.model)
                .map(str::to_string),
            reasoning_effort: evaluator_reasoning_effort(context.thinking).map(str::to_string),
            plugins: context.agent.plugins.iter().cloned().collect(),
            app_server_state_root: context.app_server_state_root.map(Path::to_path_buf),
            session_root: context.session_root.to_path_buf(),
            template_artifact_directory: context.template_artifact_directory.to_path_buf(),
            host_isolation: context.host_isolation.clone(),
            process_isolation: context.process_isolation,
            codex_executable: context.codex_executable.to_path_buf(),
        }
    }

    pub(crate) fn session_root(&self) -> &Path {
        &self.session_root
    }

    pub(crate) fn process_isolation(&self) -> EvaluatorProcessIsolation {
        self.process_isolation
    }

    /// Serializes the named permission profile passed to the Codex app-server.
    pub(crate) fn to_json_value(&self) -> EvaluatorConfigResult<Value> {
        // Git-backed scope and ignore filtering is enforced by the materialized
        // evaluator tree; in-place mode instead uses the checked directory, where
        // those filters are invalid. Permissions sandbox the resulting evaluator
        // cwd and therefore must not encode scoped project paths.
        let mut extra_permissions = BTreeMap::new();
        if self.process_isolation == EvaluatorProcessIsolation::CanonManaged {
            merge_filesystem_permissions(
                &mut extra_permissions,
                self.host_isolation.read_denials(&[
                    &self.codex_executable,
                    &self.session_root,
                    &self.template_artifact_directory,
                ])?,
            )?;
            merge_filesystem_permissions(
                &mut extra_permissions,
                evaluator_working_tree_read_exception(&self.session_root)?,
            )?;
            if let Some(app_server_state_root) = self
                .app_server_state_root
                .as_ref()
                .filter(|state_root| state_root.starts_with(&self.session_root))
            {
                merge_filesystem_permissions(
                    &mut extra_permissions,
                    evaluator_resolved_state_dir_permissions(app_server_state_root)?,
                )?;
            }
            merge_filesystem_permissions(
                &mut extra_permissions,
                evaluator_template_artifact_permissions(&self.template_artifact_directory)?,
            )?;
        }
        EvaluatorRuntimeSettings::new(
            FILESYSTEM_READ,
            self.reasoning_effort.as_deref(),
            &self.codex_executable,
        )
        .with_extra_filesystem_permissions(extra_permissions)
        .with_process_isolation(self.process_isolation)
        .with_model(self.model.as_deref())
        .with_plugins(self.plugins.iter().map(String::as_str))
        .to_json_value()
    }
}

#[cfg(test)]
mod tests;
