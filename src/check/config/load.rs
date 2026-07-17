use crate::check::config::expansion::{
    expand_raw_check_config_with_requirements, CheckConfigExpansionOptions, CheckConfigSource,
    ExpandedCheckConfig,
};
use crate::check::config::in_place::InPlaceCheckConfig;
use crate::check::config::validation::{validate_ask_config, validate_check_config};
use crate::check::config::yaml_include::parse_yaml_config_with_includes;
use crate::config_types::{CheckConfig, RawCheckConfig};
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

pub(crate) fn load_check_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
) -> Result<CheckConfig, String> {
    load_check_config_with_default_agent_preset(cache, root, config_path, source, None)
}

fn load_check_config_with_default_agent_preset(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
    default_agent_preset: Option<&str>,
) -> Result<CheckConfig, String> {
    load_tree_check_config(cache, root, config_path, source, default_agent_preset, None)
}

pub(crate) fn load_ask_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
    default_agent_preset: Option<&str>,
    question: &str,
) -> Result<CheckConfig, String> {
    load_tree_check_config(
        cache,
        root,
        config_path,
        source,
        default_agent_preset,
        Some(question),
    )
}

fn load_tree_check_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    source: &TreeSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    let content = cache
        .tree_file_content(root, source, config_path)
        .map_err(|err| map_missing_default_config_error(config_path, source, err))?;
    parse_tree_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        &content,
        source.clone(),
        default_agent_preset,
        ask_question,
    )
}

pub(crate) fn load_in_place_check_config_with_default_agent_preset(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
) -> Result<InPlaceCheckConfig, String> {
    load_in_place_config(cache, root, config_path, default_agent_preset, None)
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
    config.validate_all()?;
    Ok(config.into_config())
}

fn load_in_place_config(
    cache: &mut RepoInspectionCache,
    root: &Path,
    config_path: &Path,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<InPlaceCheckConfig, String> {
    let content = cache.in_place_file_content(root, config_path)?;
    parse_in_place_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        &content,
        default_agent_preset,
        ask_question,
    )
}

pub(super) fn parse_tree_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: TreeSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::Tree(source),
        default_agent_preset,
        ask_question,
    )?;
    validate_expanded_check_config(&expanded.config, ask_question)?;
    Ok(expanded.config)
}

fn parse_in_place_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<InPlaceCheckConfig, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::InPlace,
        default_agent_preset,
        ask_question,
    )?;
    validate_expanded_check_config(&expanded.config, ask_question)?;
    Ok(InPlaceCheckConfig::from_expanded(expanded))
}

fn expand_check_config_content(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<ExpandedCheckConfig, String> {
    // `RawCheckConfig` is the serde schema for the whole check.yml file.
    // Expansion resolves either its configured expectations or one ask-owned
    // temporary item before validation and command execution.
    let raw = parse_raw_check_config(root, config_path, content, source.clone())?;
    expand_raw_check_config_with_requirements(
        raw,
        CheckConfigExpansionOptions {
            default_agent_preset,
            ask_question,
        },
    )
}

fn validate_expanded_check_config(
    config: &CheckConfig,
    ask_question: Option<&str>,
) -> Result<(), String> {
    if ask_question.is_some() {
        validate_ask_config(config)?;
    } else {
        validate_check_config(config)?;
    }
    Ok(())
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

fn map_missing_default_config_error(
    config_path: &Path,
    source: &TreeSource,
    err: String,
) -> String {
    let missing_staged_path = format!(
        "failed to read staged {}: path is not in the staged index",
        config_path.display()
    );
    if config_path == Path::new(super::CHECK_PATH)
        && matches!(source, TreeSource::Staged)
        && err == missing_staged_path
    {
        return format!(
            "No canon check config found at {}\n\
             ▷ If the human asked you to use any `canon-*` skill in this project, \
             treat the missing config as permission to set up canon rather than asking \
             for confirmation. Run `canon init`, inspect the current project enough to \
             understand its purpose, public behavior, and important invariants, then \
             draft a minimal `.canon/check.yml` that the human can grow over time. Use \
             canon's own `.canon/check.yml` as a reference for structure and style: \
             `https://raw.githubusercontent.com/progbeat/canon/main/.canon/check.yml`. \
             Start with a few simple, objective expectations that protect important \
             user-facing behavior.",
            super::CHECK_PATH
        );
    }
    err
}
