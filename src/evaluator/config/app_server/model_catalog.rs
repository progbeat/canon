use super::super::{path_to_config_string, EvaluatorConfigError, EvaluatorConfigResult};
use crate::memoize::{mutex_memoized_result, MemoizedResult};
use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const MODEL_CATALOG_FILE_NAME: &str = "models.json";
const MODEL_CATALOG_ARTIFACT_PREFIX: &str = "canon-evaluator-model-catalog-artifact";

static BUNDLED_MODEL_CATALOGS: OnceLock<Mutex<BTreeMap<PathBuf, MemoizedResult<String>>>> =
    OnceLock::new();

pub(super) struct EvaluatorModelCatalogArtifact {
    _artifact_directory: OwnedPrivateTemporaryDirectory,
    path: PathBuf,
}

pub(super) fn materialize_evaluator_model_catalog_artifact(
    codex_executable: &Path,
) -> EvaluatorConfigResult<EvaluatorModelCatalogArtifact> {
    let catalog = bundled_model_catalog(codex_executable)?;
    let artifact_directory = OwnedPrivateTemporaryDirectory::create(
        &PrivateTemporaryDirectoryAllocator::new(),
        MODEL_CATALOG_ARTIFACT_PREFIX,
    )
    .map_err(EvaluatorConfigError::Message)?;
    let path = artifact_directory.path().join(MODEL_CATALOG_FILE_NAME);
    // Codex's public `model_catalog_json` contract accepts a path, so this
    // immutable startup input must be materialized for the child to read. The
    // owning artifact is removed with the app-server; canon never reads it
    // back as invocation state.
    fs::write(&path, catalog).map_err(|err| {
        EvaluatorConfigError::Message(format!(
            "failed to materialize evaluator model catalog artifact {}: {err}",
            path.display()
        ))
    })?;
    Ok(EvaluatorModelCatalogArtifact {
        _artifact_directory: artifact_directory,
        path,
    })
}

fn bundled_model_catalog(codex_executable: &Path) -> EvaluatorConfigResult<String> {
    let state = BUNDLED_MODEL_CATALOGS.get_or_init(|| Mutex::new(BTreeMap::new()));
    mutex_memoized_result(
        state,
        codex_executable.to_path_buf(),
        "evaluator model catalog cache is poisoned",
        |entries| entries,
        |entries| entries,
        || read_bundled_model_catalog(codex_executable),
    )
    .map_err(EvaluatorConfigError::Message)
}

fn read_bundled_model_catalog(codex_executable: &Path) -> Result<String, String> {
    // `codex debug models --bundled` is Codex's public, version-matched model
    // catalog interface. Preserve that catalog instead of duplicating its
    // evolving schema, changing only the capability the read-only evaluator
    // must not receive.
    let output = Command::new(codex_executable)
        .args(["debug", "models", "--bundled"])
        .output()
        .map_err(|err| {
            format!(
                "failed to read bundled model catalog from {}: {err}",
                codex_executable.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "{} debug models --bundled failed with {}: {}",
            codex_executable.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    disable_apply_patch_tool(&output.stdout)
}

fn disable_apply_patch_tool(catalog: &[u8]) -> Result<String, String> {
    let mut catalog: Value = serde_json::from_slice(catalog)
        .map_err(|err| format!("failed to parse bundled model catalog: {err}"))?;
    let models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "bundled model catalog is missing models".to_string())?;
    if models.is_empty() {
        return Err("bundled model catalog contains no models".to_string());
    }
    for model in models {
        let model = model
            .as_object_mut()
            .ok_or_else(|| "bundled model catalog contains a non-object model".to_string())?;
        model.insert("apply_patch_tool_type".to_string(), Value::Null);
    }
    serde_json::to_string(&catalog).map_err(|err| format!("failed to encode model catalog: {err}"))
}

impl EvaluatorModelCatalogArtifact {
    pub(super) fn config_arg(&self) -> EvaluatorConfigResult<String> {
        let path = path_to_config_string(&self.path, "evaluator model catalog path")?;
        Ok(format!(
            "model_catalog_json={}",
            super::super::codec::toml_string(&path)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test] // xpec: H5,hQ
    fn bundled_catalog_metadata_is_preserved_while_apply_patch_is_disabled() {
        let catalog = json!({
            "models": [{
                "slug": "model",
                "description": "current metadata",
                "apply_patch_tool_type": "freeform"
            }]
        });

        let transformed = disable_apply_patch_tool(catalog.to_string().as_bytes()).unwrap();
        let transformed: Value = serde_json::from_str(&transformed).unwrap();

        assert_eq!(transformed["models"][0]["slug"], json!("model"));
        assert_eq!(
            transformed["models"][0]["description"],
            json!("current metadata")
        );
        assert_eq!(
            transformed["models"][0]["apply_patch_tool_type"],
            Value::Null
        );
    }
}
