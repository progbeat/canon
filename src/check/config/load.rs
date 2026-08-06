mod content;
mod in_place;
mod tree;

pub(crate) use in_place::{
    collect_in_place_check_config_with_default_agent_preset, load_in_place_ask_config,
};
pub(crate) use tree::{collect_check_config, load_ask_config, load_check_config};

#[cfg(test)]
pub(super) use content::parse_tree_check_config_content_with_root_and_default_agent_preset;
#[cfg(test)]
use in_place::map_missing_in_place_default_config_error;
#[cfg(test)]
use tree::map_missing_default_config_error;

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

pub(super) fn missing_default_config_error() -> String {
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

pub(crate) fn is_missing_default_config_error(error: &str) -> bool {
    error == missing_default_config_error()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::TreeSource;
    use crate::repo_inspection::RepoInspectionCache;
    use std::fs;
    use std::path::Path;

    // xpec: D8
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

    #[test] // xpec: D8
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
