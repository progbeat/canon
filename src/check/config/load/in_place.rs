use super::content::collect_in_place_check_config_content_with_root_and_default_agent_preset;
use super::{missing_default_config_error, CollectedCheckConfig};
use crate::check::config::in_place::InPlaceCheckConfig;
use crate::config_types::CheckConfig;
use crate::fs_util::path_exists_no_follow;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

pub(crate) fn collect_in_place_check_config_with_default_agent_preset(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
) -> Result<CollectedCheckConfig<InPlaceCheckConfig>, String> {
    collect_in_place_config(cache, root, config_path, default_agent_preset, None)
}

pub(crate) fn load_in_place_ask_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
    question: &str,
) -> Result<CheckConfig, String> {
    let config = load_in_place_config(
        cache,
        root,
        config_path,
        default_agent_preset,
        Some(question),
    )?;
    config.validate_configured_fields()?;
    Ok(config.into_config())
}

fn load_in_place_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<InPlaceCheckConfig, String> {
    collect_in_place_config(cache, root, config_path, default_agent_preset, ask_question)?
        .into_validated()
}

fn collect_in_place_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<InPlaceCheckConfig>, String> {
    let content = cache
        .in_place_file_content(root, config_path)
        .map_err(|err| map_missing_in_place_default_config_error(root, config_path, err))?;
    collect_in_place_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        &content,
        cache.clone(),
        default_agent_preset,
        ask_question,
    )
}

pub(super) fn map_missing_in_place_default_config_error(
    root: &Path,
    config_path: &Path,
    err: String,
) -> String {
    if config_path == Path::new(super::super::CHECK_PATH)
        && matches!(path_exists_no_follow(&root.join(config_path)), Ok(false))
    {
        return missing_default_config_error();
    }
    err
}
