use super::codec::toml_string;
use super::{path_to_config_string, EvaluatorConfigError, EvaluatorConfigResult};
use crate::config_types::AgentConfig;
use crate::fs_util::write_temp_file_then_replace;
use crate::platform;
use serde::Serialize;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const EVALUATOR_MODEL_CATALOG_TEMP_DIR: &str = "canon-evaluator-model-catalogs";
static MODEL_CATALOG_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) struct ModelCatalogFile {
    path: PathBuf,
}

pub(super) struct ModelCatalogConfigArg {
    pub(super) arg: String,
    pub(super) file: ModelCatalogFile,
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

pub(super) fn evaluator_model_catalog_config_arg(
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

fn evaluator_model_catalog_json(models: &[String]) -> EvaluatorConfigResult<String> {
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

impl ModelCatalogFile {
    fn new(path: PathBuf) -> ModelCatalogFile {
        ModelCatalogFile { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ModelCatalogFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
