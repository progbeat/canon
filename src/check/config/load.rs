use crate::check::config::expansion::{
    expand_raw_check_config_with_requirements, CheckConfigExpansionOptions, CheckConfigSource,
    ExpandedCheckConfig,
};
use crate::check::config::in_place::try_into_in_place_config;
use crate::check::config::validation::{validate_ask_config, validate_check_config};
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
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        cache,
        CheckConfigSource::Tree(source),
        default_agent_preset,
        ask_question,
    )?;
    validate_expanded_check_config(expanded.config, ask_question)
}

pub(crate) fn parse_in_place_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        cache,
        CheckConfigSource::InPlace,
        default_agent_preset,
        ask_question,
    )?;
    let config = try_into_in_place_config(expanded)?;
    validate_expanded_check_config(config, ask_question)
}

fn expand_check_config_content(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
    source: CheckConfigSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<ExpandedCheckConfig, String> {
    // `RawCheckConfig` is the serde schema for the whole check.yml file.
    // Expansion resolves either its configured expectations or one ask-owned
    // temporary item before validation and command execution.
    let raw = parse_raw_check_config(root, config_path, content, source.clone())?;
    expand_raw_check_config_with_requirements(
        Some(root),
        config_path,
        raw,
        Some(cache),
        source,
        CheckConfigExpansionOptions {
            default_agent_preset,
            ask_question,
        },
    )
}

fn validate_expanded_check_config(
    config: CheckConfig,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    if ask_question.is_some() {
        validate_ask_config(&config)?;
    } else {
        validate_check_config(&config)?;
    }
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
