use crate::check::config::config_expansion::{
    expand_raw_check_config_with_options, CheckConfigExpansionOptions, CheckConfigSource,
};
use crate::check::config::validation::validate_check_config;
use crate::check::config::yaml_include::parse_yaml_config_with_includes;
use crate::config_types::{CheckConfig, RawCheckConfig};
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

pub(crate) fn parse_tree_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
    source: TreeSource,
    default_agent_preset: Option<&str>,
) -> Result<CheckConfig, String> {
    parse_check_config_content_with_root_and_source_and_default_agent_preset(
        root,
        config_path,
        content,
        cache,
        CheckConfigSource::Tree(source),
        default_agent_preset,
    )
}

pub(crate) fn parse_check_config_content_with_root_and_source_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
    source: CheckConfigSource,
    default_agent_preset: Option<&str>,
) -> Result<CheckConfig, String> {
    // `RawCheckConfig` is the serde schema for the whole check.yml file,
    // including optional top-level `canon check` hooks. Expansion resolves
    // those hooks into `CheckConfig.hooks` before validation or command
    // execution; Git pre-commit hook installation is a separate module.
    let raw = parse_raw_check_config(root, config_path, content, source.clone())?;
    let config = expand_raw_check_config_with_options(
        Some(root),
        config_path,
        raw,
        Some(cache),
        source,
        CheckConfigExpansionOptions {
            default_agent_preset,
        },
    )?;
    validate_check_config(&config)?;
    Ok(config)
}

fn parse_raw_check_config(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
) -> Result<RawCheckConfig, String> {
    parse_yaml_config_with_includes(root, config_path, content, source)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))
}
