use crate::check::config::expansion::{
    expand_raw_check_config_for_command, CheckConfigExpansionOptions, CheckConfigSource,
};
use crate::check::config::in_place::InPlaceCheckConfig;
use crate::check::config::validation::{validate_ask_config, validate_check_config};
use crate::check::config::yaml_include::parse_yaml_config_with_includes;
use crate::config_types::{CheckConfig, RawCheckConfig};
use crate::fs_util::path_exists_no_follow;
use crate::git::TreeSource;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

pub(crate) struct CollectedCheckConfig<T> {
    expectation_count: usize,
    validation: Result<T, String>,
}

impl<T> CollectedCheckConfig<T> {
    pub(crate) fn expectation_count(&self) -> usize {
        self.expectation_count
    }

    pub(crate) fn into_validated(self) -> Result<T, String> {
        self.validation
    }
}

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
        default_agent_preset,
        ask_question,
    )
}

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
    collect_tree_check_config_content_with_root_and_default_agent_preset(
        root,
        config_path,
        content,
        source,
        default_agent_preset,
        ask_question,
    )?
    .into_validated()
}

fn collect_tree_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    source: TreeSource,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<CheckConfig>, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::Tree(source),
        default_agent_preset,
        ask_question,
    )?;
    let expectation_count = expanded.expectations.len();
    let validation = validate_expanded_check_config(&expanded, ask_question).map(|()| expanded);
    Ok(CollectedCheckConfig {
        expectation_count,
        validation,
    })
}

fn collect_in_place_check_config_content_with_root_and_default_agent_preset(
    root: &Path,
    config_path: &Path,
    content: &str,
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CollectedCheckConfig<InPlaceCheckConfig>, String> {
    let expanded = expand_check_config_content(
        root,
        config_path,
        content,
        CheckConfigSource::InPlace,
        default_agent_preset,
        ask_question,
    )?;
    let expectation_count = expanded.expectations.len();
    let validation = validate_expanded_check_config(&expanded, ask_question)
        .map(|()| InPlaceCheckConfig::from_config(expanded));
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
    default_agent_preset: Option<&str>,
    ask_question: Option<&str>,
) -> Result<CheckConfig, String> {
    // `RawCheckConfig` is the serde schema for the whole check.yml file.
    // Expansion resolves both configured items and the optional ask-owned
    // temporary runtime item through one preset-resolution boundary.
    let in_place = matches!(&source, CheckConfigSource::InPlace);
    let raw = parse_raw_check_config(root, config_path, content, source.clone())?;
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
) -> Result<RawCheckConfig, String> {
    parse_yaml_config_with_includes(root, config_path, content, source)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))
}

fn map_missing_default_config_error(
    config_path: &Path,
    source: &TreeSource,
    err: String,
) -> String {
    let missing_path_error = match source {
        TreeSource::Staged => format!(
            "failed to read staged {}: path is not in the staged index",
            config_path.display()
        ),
        TreeSource::Git { .. } | TreeSource::DefaultAgainstHead { .. } => format!(
            "failed to read {} from {}: path is not in the selected tree",
            config_path.display(),
            source.cache_key()
        ),
    };
    if config_path == Path::new(super::CHECK_PATH) && err == missing_path_error {
        return missing_default_config_error();
    }
    err
}

fn map_missing_in_place_default_config_error(
    root: &Path,
    config_path: &Path,
    err: String,
) -> String {
    if config_path == Path::new(super::CHECK_PATH)
        && matches!(path_exists_no_follow(&root.join(config_path)), Ok(false))
    {
        return missing_default_config_error();
    }
    err
}

fn missing_default_config_error() -> String {
    format!(
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // xpec: Y8
    #[test]
    fn missing_default_staged_config_has_setup_guidance() {
        let error = map_missing_default_config_error(
            Path::new(super::super::CHECK_PATH),
            &TreeSource::Staged,
            "failed to read staged .canon/check.yml: path is not in the staged index".into(),
        );

        assert!(error.starts_with("No canon check config found at .canon/check.yml\n"));
        assert!(error.contains("Run `canon init`"));
        assert!(error.contains("draft a minimal `.canon/check.yml`"));
    }

    #[test] // xpec: Y8
    fn missing_default_in_place_config_has_setup_guidance() {
        let root = std::env::temp_dir().join(format!(
            "canon-missing-in-place-config-{}-{:016x}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut cache = RepoInspectionCache::new();

        let error = match collect_in_place_check_config_with_default_agent_preset(
            &mut cache,
            &root,
            Path::new(super::super::CHECK_PATH),
            None,
        ) {
            Ok(_) => panic!("missing default config must fail"),
            Err(error) => error,
        };

        assert!(error.starts_with("No canon check config found at .canon/check.yml\n"));
        assert!(error.contains("Run `canon init`"));
        fs::remove_dir_all(root).unwrap();
    }
}
