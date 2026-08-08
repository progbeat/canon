use super::content::collect_tree_check_config_content_with_root_and_default_agent_preset;
use super::{missing_default_config_error, CollectedCheckConfig};
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

pub(crate) fn load_check_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
) -> Result<CheckConfig, String> {
    collect_check_config(cache, root, config_path, source)?.into_validated()
}

pub(crate) fn collect_check_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
) -> Result<CollectedCheckConfig<CheckConfig>, String> {
    collect_tree_check_config(cache, root, config_path, source, None, None)
}

pub(crate) fn load_ask_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
    default_agent_preset: Option<&str>,
    question: &str,
) -> Result<CheckConfig, String> {
    collect_tree_check_config(
        cache,
        root,
        config_path,
        source,
        default_agent_preset,
        Some(question),
    )?
    .into_validated()
}

fn collect_tree_check_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<CheckConfig>, String> {
    let content = cache
        .tree_file_content(root, source, config_path)
        .map_err(|err| map_missing_default_config_error(config_path, source, err))?;
    collect_tree_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        &content,
        source.clone(),
        cache.clone(),
        default_agent_preset,
        ask_question,
    )
}

pub(super) fn map_missing_default_config_error(
    config_path: &Path,
    source: &TreeSource,
    err: String,
) -> String {
    let missing_path_error = match source {
        TreeSource::Staged => format!(
            "failed to read staged {}: path is not in the staged index",
            config_path.display()
        ),
        TreeSource::Git { .. }
        | TreeSource::TemporaryGit { .. }
        | TreeSource::DefaultAgainstHead { .. }
        | TreeSource::DefaultAgainstUnbornHead { .. } => format!(
            "failed to read {} from {}: path is not in the selected tree",
            config_path.display(),
            source.cache_key()
        ),
    };
    if config_path == Path::new(super::super::CHECK_PATH) && err == missing_path_error {
        return missing_default_config_error();
    }
    err
}
