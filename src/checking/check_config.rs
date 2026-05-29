use crate::check_config_expansion::{expand_raw_check_config, CheckConfigSource};
use crate::check_validation::validate_check_config;
use crate::config_types::{CheckConfig, RawCheckConfig};
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

#[cfg(test)]
use crate::CHECK_PATH;

#[cfg(test)]
pub(crate) fn parse_check_config_content(
    config_path: &Path,
    content: &str,
) -> Result<CheckConfig, String> {
    let raw = parse_raw_check_config(config_path, content)?;
    let config =
        expand_raw_check_config(None, config_path, raw, None, CheckConfigSource::Worktree)?;
    validate_check_config(&config)?;
    Ok(config)
}

#[cfg(test)]
pub(crate) fn parse_check_config_content_with_root(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
) -> Result<CheckConfig, String> {
    parse_check_config_content_with_root_and_source(
        root,
        config_path,
        content,
        cache,
        if config_path == Path::new(CHECK_PATH) {
            CheckConfigSource::Staged
        } else {
            CheckConfigSource::Worktree
        },
    )
}

pub(crate) fn parse_staged_check_config_content_with_root(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
) -> Result<CheckConfig, String> {
    parse_check_config_content_with_root_and_source(
        root,
        config_path,
        content,
        cache,
        CheckConfigSource::Staged,
    )
}

fn parse_check_config_content_with_root_and_source(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
    source: CheckConfigSource,
) -> Result<CheckConfig, String> {
    let raw = parse_raw_check_config(config_path, content)?;
    let config = expand_raw_check_config(Some(root), config_path, raw, Some(cache), source)?;
    validate_check_config(&config)?;
    Ok(config)
}

fn parse_raw_check_config(config_path: &Path, content: &str) -> Result<RawCheckConfig, String> {
    serde_saphyr::from_str(content)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))
}
