use super::base::push_evaluator_startup_config_args;
use super::codec::push_config_arg;
use super::model_catalog::{evaluator_model_catalog_config_arg, ModelCatalogFile};
use super::{
    EvaluatorConfigResult, EVALUATOR_DISABLED_FEATURES, EVALUATOR_EXTRA_DISABLED_FEATURES,
};
use crate::config_types::AgentConfig;
use std::path::Path;

pub(crate) struct AppServerArgs {
    pub(crate) args: Vec<String>,
    pub(crate) model_catalog_file: Option<ModelCatalogFile>,
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
    _root: &Path,
    agent: &AgentConfig,
    no_sandbox: bool,
) -> EvaluatorConfigResult<StartupConfigArgs> {
    let mut args = Vec::new();
    let mut model_catalog_file = None;
    if no_sandbox {
        // Docker supplies the outer isolation boundary. Keep Canon's
        // permission profile below so evaluator tools are still confined to
        // the materialized snapshot, while avoiding the host OS sandbox
        // launcher that is unavailable in the container.
        push_config_arg(&mut args, "sandbox_mode=\"danger-full-access\"");
    }
    push_evaluator_startup_config_args(&mut args, agent)?;
    if let Some(model_catalog_arg) = evaluator_model_catalog_config_arg(agent)? {
        push_config_arg(&mut args, &model_catalog_arg.arg);
        model_catalog_file = Some(model_catalog_arg.file);
    }
    Ok(StartupConfigArgs {
        args,
        model_catalog_file,
    })
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
