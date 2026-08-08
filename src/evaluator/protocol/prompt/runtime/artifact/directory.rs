use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX: &str = "canon-prompt-template-artifact";

pub(crate) struct PromptTemplateArtifactDirCache {
    artifact_directory: OnceLock<OwnedPrivateTemporaryDirectory>,
    stdout_artifact_paths: Mutex<BTreeMap<Vec<u8>, PathBuf>>,
    temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
}

#[derive(Clone)]
pub(crate) enum PromptTemplateArtifactDir {
    Lazy(Arc<PromptTemplateArtifactDirCache>),
    #[cfg(test)]
    Fixed(PathBuf),
}

impl PromptTemplateArtifactDirCache {
    pub(crate) fn new(
        temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
    ) -> PromptTemplateArtifactDirCache {
        PromptTemplateArtifactDirCache {
            artifact_directory: OnceLock::new(),
            stdout_artifact_paths: Mutex::new(BTreeMap::new()),
            temporary_directory_allocator,
        }
    }

    pub(crate) fn path_for_prompt_artifacts(&self) -> Result<PathBuf, String> {
        if let Some(dir) = self.artifact_directory.get() {
            return Ok(dir.path().to_path_buf());
        }
        let dir = allocate_prompt_template_artifact_dir(&self.temporary_directory_allocator)?;
        let path = dir.path().to_path_buf();
        if self.artifact_directory.set(dir).is_err() {
            return Ok(self
                .artifact_directory
                .get()
                .expect("prompt template artifact dir is set")
                .path()
                .to_path_buf());
        }
        Ok(path)
    }

    pub(super) fn materialize_stdout_artifact(
        &self,
        stdout: &[u8],
        path_for_content: impl FnOnce(&Path) -> PathBuf,
        materialize: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<PathBuf, String> {
        let mut paths = self
            .stdout_artifact_paths
            .lock()
            .map_err(|_| "prompt template stdout artifact path cache is poisoned".to_string())?;
        if let Some(path) = paths.get(stdout) {
            return Ok(path.clone());
        }
        let artifact_dir = self.path_for_prompt_artifacts()?;
        let path = path_for_content(&artifact_dir);
        materialize(&path)?;
        paths.insert(stdout.to_vec(), path.clone());
        Ok(path)
    }
}

pub(crate) fn allocate_prompt_template_artifact_dir(
    temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
) -> Result<OwnedPrivateTemporaryDirectory, String> {
    // [g2] The OnceLock and content-to-path map above are the invocation-local
    // state and remain in memory. Prompt Templates require oversized command
    // output to be exposed as evaluator-readable files; this owned directory
    // contains only those interface payloads, which Canon never reads back to
    // decide a check result. The cache shares it across prompt renders.
    OwnedPrivateTemporaryDirectory::create(
        temporary_directory_allocator,
        PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX,
    )
}

#[cfg(test)]
mod tests;
