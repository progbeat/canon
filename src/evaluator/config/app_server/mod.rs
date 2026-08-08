mod model_catalog;

use super::codec::push_config_arg;
use super::runtime::push_evaluator_startup_config_args;
use super::{EvaluatorConfigResult, EVALUATOR_DISABLED_FEATURES};
use crate::config_types::AgentConfig;
use crate::evaluator::ReadOnlyProjectInspectionPlan;
use model_catalog::{materialize_evaluator_model_catalog_artifact, EvaluatorModelCatalogArtifact};
use std::path::Path;

pub(crate) struct AppServerArgs {
    args: Vec<String>,
    _model_catalog_artifact: EvaluatorModelCatalogArtifact,
}

pub(crate) fn app_server_args(
    runtime_codex_executable: &Path,
    installed_codex_executable: &Path,
    load_plugins: bool,
    agent: &AgentConfig,
    inspection_plan: &ReadOnlyProjectInspectionPlan,
) -> EvaluatorConfigResult<AppServerArgs> {
    let mut args = vec!["app-server".to_string()];
    for feature in evaluator_disabled_app_server_features(load_plugins) {
        args.push("--disable".to_string());
        args.push(feature.to_string());
    }
    push_evaluator_startup_config_args(
        &mut args,
        agent,
        inspection_plan.process_isolation(),
        runtime_codex_executable,
    )?;
    let model_catalog_artifact =
        materialize_evaluator_model_catalog_artifact(installed_codex_executable)?;
    push_config_arg(&mut args, &model_catalog_artifact.config_arg()?);
    args.push("--listen".to_string());
    args.push("stdio://".to_string());
    Ok(AppServerArgs {
        args,
        _model_catalog_artifact: model_catalog_artifact,
    })
}

impl AppServerArgs {
    pub(crate) fn args(&self) -> &[String] {
        &self.args
    }
}

fn evaluator_disabled_app_server_features(load_plugins: bool) -> Vec<&'static str> {
    let mut features = Vec::new();
    if !load_plugins {
        features.push("plugins");
    }
    // [bP,KD,hQ,l] Canon's built-in evaluator project access uses only the
    // read-only dynamic tool plan. Process-isolation mode never enables the
    // app-server's filesystem-writing shell capability.
    features.push("shell_tool");
    features.extend(EVALUATOR_DISABLED_FEATURES.iter().copied());
    features
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: bP,KD,l
    fn evaluator_shell_tool_is_always_disabled() {
        assert!(evaluator_disabled_app_server_features(false).contains(&"shell_tool"));
        assert!(evaluator_disabled_app_server_features(true).contains(&"shell_tool"));
    }
}
