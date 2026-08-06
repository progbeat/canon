use super::CollectedCheckConfig;
use crate::check::config::expansion::{
    expand_raw_check_config_for_command, CheckConfigExpansionOptions, CheckConfigSource,
};
use crate::check::config::in_place::InPlaceCheckConfig;
use crate::check::config::validation::{validate_ask_config, validate_check_config};
use crate::check::config::yaml_include::parse_raw_check_config_with_includes_and_foreach;
use crate::config_types::{CheckConfig, RawCheckConfig};
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

#[cfg(test)]
pub(in crate::check::config) fn parse_tree_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: TreeSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    collect_tree_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        content,
        source,
        RepoInspectionCache::new(),
        default_agent_preset,
        ask_question,
    )?
    .into_validated()
}

pub(super) fn collect_tree_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: TreeSource,
    inspection_cache: RepoInspectionCache,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<CheckConfig>, String> {
    collect_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::Tree(source),
        inspection_cache,
        default_agent_preset,
        ask_question,
        |config| config,
    )
}

pub(super) fn collect_in_place_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    inspection_cache: RepoInspectionCache,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<InPlaceCheckConfig>, String> {
    collect_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::InPlace,
        inspection_cache,
        default_agent_preset,
        ask_question,
        InPlaceCheckConfig::from_config,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_check_config_content<T>(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    inspection_cache: RepoInspectionCache,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
    into_validated: impl FnOnce(CheckConfig) -> T,
) -> Result<CollectedCheckConfig<T>, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        source,
        inspection_cache,
        default_agent_preset,
        ask_question,
    )?;
    let expectation_count = expanded.expectations.len();
    let validation =
        validate_expanded_check_config(&expanded, ask_question).map(|()| into_validated(expanded));
    Ok(CollectedCheckConfig {
        expectation_count,
        validation,
    })
}

fn expand_check_config_content(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: CheckConfigSource,
    inspection_cache: RepoInspectionCache,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    // `RawCheckConfig` is the serde schema for the whole check.yml file.
    // Check expands configured items. Ask instead expands only its canonical
    // runtime xpec after any required raw in-place compatibility inspection.
    let in_place = matches!(&source, CheckConfigSource::InPlace);
    let raw = parse_raw_check_config(root, config_path, content, source.clone(), inspection_cache)?;
    expand_raw_check_config_for_command(
        raw,
        CheckConfigExpansionOptions {
            default_agent_preset,
            ask_question,
            in_place,
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
    inspection_cache: RepoInspectionCache,
) -> Result<RawCheckConfig, String> {
    parse_raw_check_config_with_includes_and_foreach(
        root,
        config_path,
        content,
        source,
        inspection_cache,
    )
    .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))
}
