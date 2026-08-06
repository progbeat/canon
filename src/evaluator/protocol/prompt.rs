mod developer;
mod runtime;
mod shell;
mod template;
mod turn;

use crate::platform::filesystem::PrivateTemporaryDirectoryAllocator;
use developer::render_developer_instructions_with_artifacts;
pub(crate) use developer::{developer_instructions_cache_key, DeveloperInstructionsCacheKey};
use minijinja::Environment;
use runtime::{PromptTemplateArtifactDir, PromptTemplateArtifactDirCache};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use template::prompt_template_environment;
use turn::render_evaluator_turn_prompt_with_artifacts;

pub(crate) struct DeveloperInstructionsContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) mode: EvaluatorPromptMode<'a>,
    // Data for the resource template's `xpec.instructions` variable.
    pub(crate) question_context: &'a str,
    pub(crate) visible_scope: &'a [String],
    pub(crate) num_invisible_files: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum EvaluatorPromptMode<'a> {
    // [hj,90] Encoding the target only in the Git-backed variant makes the
    // contradictory in-place + target-diff instruction set unrepresentable.
    InPlace,
    GitDiff {
        target_is_diff: bool,
        base_tree_oid: &'a str,
        checked_tree_oid: &'a str,
        git_environment: &'a [(OsString, OsString)],
    },
}

impl<'a> EvaluatorPromptMode<'a> {
    pub(crate) fn git_diff_tree_oids(self) -> Option<(&'a str, &'a str)> {
        match self {
            EvaluatorPromptMode::InPlace => None,
            EvaluatorPromptMode::GitDiff {
                base_tree_oid,
                checked_tree_oid,
                ..
            } => Some((base_tree_oid, checked_tree_oid)),
        }
    }

    pub(crate) fn target_is_diff(self) -> bool {
        match self {
            EvaluatorPromptMode::InPlace => false,
            EvaluatorPromptMode::GitDiff { target_is_diff, .. } => target_is_diff,
        }
    }
}

fn git_diff_shell_environment(
    base_tree_oid: &str,
    checked_tree_oid: &str,
    git_environment: &[(OsString, OsString)],
) -> Vec<(OsString, OsString)> {
    [
        (OsString::from("BASE_TREE"), OsString::from(base_tree_oid)),
        (
            OsString::from("CHECKED_TREE"),
            OsString::from(checked_tree_oid),
        ),
    ]
    .into_iter()
    .chain(git_environment.iter().cloned())
    .collect()
}

pub(crate) struct EvaluatorTurnPromptContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) short_id: &'a str,
    pub(crate) question: &'a str,
    pub(crate) mode: EvaluatorPromptMode<'a>,
}

pub(crate) struct RenderedPrompt {
    pub(crate) text: String,
}

pub(crate) struct PromptRenderer {
    artifact_dir_cache: Arc<PromptTemplateArtifactDirCache>,
    environment: Result<Environment<'static>, String>,
    developer_instructions_cache:
        Mutex<BTreeMap<DeveloperInstructionsCacheKey, Result<String, String>>>,
}

impl PromptRenderer {
    pub(crate) fn new(
        temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
    ) -> PromptRenderer {
        PromptRenderer {
            artifact_dir_cache: Arc::new(PromptTemplateArtifactDirCache::new(
                temporary_directory_allocator,
            )),
            environment: prompt_template_environment(),
            developer_instructions_cache: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn artifact_directory(&self) -> Result<PathBuf, String> {
        self.artifact_dir_cache.path_for_prompt_artifacts()
    }

    pub(crate) fn developer_instructions(
        &self,
        context: DeveloperInstructionsContext<'_>,
    ) -> Result<RenderedPrompt, String> {
        let cache_key = developer_instructions_cache_key(&context);
        if let Some(cached) = self
            .developer_instructions_cache
            .lock()
            .map_err(|_| "developer instructions cache lock is poisoned".to_string())?
            .get(&cache_key)
            .cloned()
        {
            return cached.map(|text| RenderedPrompt { text });
        }
        let text = render_with_lazy_artifacts(
            self.environment()?,
            &self.artifact_dir_cache,
            context,
            render_developer_instructions_with_artifacts,
        );
        self.developer_instructions_cache
            .lock()
            .map_err(|_| "developer instructions cache lock is poisoned".to_string())?
            .insert(cache_key, text.clone());
        text.map(|text| RenderedPrompt { text })
    }

    pub(crate) fn evaluator_turn_prompt(
        &self,
        context: EvaluatorTurnPromptContext<'_>,
    ) -> Result<RenderedPrompt, String> {
        // [d] The turn template is input-specific and performs no repository
        // inspection or other expensive deterministic operation.
        let text = render_with_lazy_artifacts(
            self.environment()?,
            &self.artifact_dir_cache,
            context,
            render_evaluator_turn_prompt_with_artifacts,
        )?;
        Ok(RenderedPrompt { text })
    }

    fn environment(&self) -> Result<&Environment<'static>, String> {
        self.environment.as_ref().map_err(Clone::clone)
    }
}

fn render_with_lazy_artifacts<Context>(
    environment: &Environment<'_>,
    artifact_dir_cache: &Arc<PromptTemplateArtifactDirCache>,
    context: Context,
    render: impl FnOnce(
        &Environment<'_>,
        PromptTemplateArtifactDir,
        &mut Vec<PathBuf>,
        Context,
    ) -> Result<String, String>,
) -> Result<String, String> {
    let mut artifact_paths = Vec::new();
    render(
        environment,
        PromptTemplateArtifactDir::Lazy(Arc::clone(artifact_dir_cache)),
        &mut artifact_paths,
        context,
    )
}
