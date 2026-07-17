//! Applies only the additional `canon check --in-place` config restrictions.
//!
//! Expansion and general config validation run before this wrapper is
//! constructed. Git-backed checks use that validated config directly. Only
//! in-place commands call the methods below to apply their separate mode
//! contract to the complete configured field set.

use super::expansion::{ExpandedCheckConfig, InPlaceRequirements};
use crate::config_types::CheckConfig;

#[derive(Clone)]
pub(crate) struct InPlaceCheckConfig {
    config: CheckConfig,
    requirements: InPlaceRequirements,
}

impl InPlaceCheckConfig {
    pub(crate) fn from_expanded(expanded: ExpandedCheckConfig) -> InPlaceCheckConfig {
        InPlaceCheckConfig {
            config: expanded.config,
            requirements: expanded.in_place_requirements,
        }
    }

    pub(crate) fn config(&self) -> &CheckConfig {
        &self.config
    }

    pub(crate) fn into_config(self) -> CheckConfig {
        self.config
    }

    pub(crate) fn validate_configured_fields(&self) -> Result<(), String> {
        // [Df] The canon separately requires selected expectations to work
        // without Git-backed behavior and prohibits these configured fields
        // throughout in-place mode, including on unselected expectations.
        if self.requirements.config_uses_ignore {
            return Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string(),
            );
        }
        for expectation in &self.requirements.git_backed_only_expectation_fields {
            if !expectation.git_backed_only_field_names.is_empty() {
                return Err(format!(
                    "expectation {} is invalid in in-place mode: {}",
                    expectation.item_number,
                    expectation
                        .git_backed_only_field_names
                        .iter()
                        .map(|field| format!("`{field}` requires Git-backed check state"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::config::expansion::{
        expand_raw_check_config_with_requirements, CheckConfigExpansionOptions,
    };
    use crate::config_types::RawCheckConfig;

    fn parse_in_place_config(yaml: &str) -> InPlaceCheckConfig {
        let raw: RawCheckConfig = serde_saphyr::from_str(yaml).expect("parse raw config");
        let expanded =
            expand_raw_check_config_with_requirements(raw, CheckConfigExpansionOptions::default())
                .expect("expand raw config");
        InPlaceCheckConfig::from_expanded(expanded)
    }

    fn parse_in_place_ask_config(yaml: &str) -> InPlaceCheckConfig {
        let raw: RawCheckConfig = serde_saphyr::from_str(yaml).expect("parse raw config");
        let expanded = expand_raw_check_config_with_requirements(
            raw,
            CheckConfigExpansionOptions {
                ask_question: Some("Does this work in place?"),
                ..CheckConfigExpansionOptions::default()
            },
        )
        .expect("expand raw ask config");
        InPlaceCheckConfig::from_expanded(expanded)
    }

    #[test] // xpec: Df,T
    fn ask_validation_retains_prohibitions_from_configured_expectations() {
        let config = parse_in_place_ask_config(
            r#"
presets:
  default: {}
expectations:
  - q: "Configured Git-backed check"
    a: "yes"
    diff-from: :against-tree
"#,
        );

        assert_eq!(config.config().expectations.len(), 1);
        assert_eq!(
            config.config().expectations[0].q,
            "Does this work in place?"
        );
        assert_eq!(
            config.validate_configured_fields(),
            Err("expectation 1 is invalid in in-place mode: \
                 `diff-from` requires Git-backed check state"
                .to_string())
        );
    }

    #[test] // xpec: Df
    fn configured_git_backed_expectation_fields_are_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default: {}
expectations:
  - q: "In-place compatible"
    a: "yes"
  - q: "Git-backed only"
    a: "yes"
    diff-from: :against-tree
"#,
        );

        assert_eq!(
            config.validate_configured_fields(),
            Err("expectation 2 is invalid in in-place mode: \
                 `diff-from` requires Git-backed check state"
                .to_string())
        );
    }

    #[test] // xpec: 6,Df
    fn cooldown_is_supported_for_git_state_but_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default: {}
expectations:
  - q: "Expensive Git-backed quality check"
    a: "yes"
    cooldown: 7d
"#,
        );

        assert_eq!(
            config.validate_configured_fields(),
            Err("expectation 1 is invalid in in-place mode: \
                 `cooldown` requires Git-backed check state"
                .to_string())
        );
    }

    #[test] // xpec: Df
    fn configured_ignore_is_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default:
    ignore: ["tmp/**"]
expectations:
  - q: "In-place compatible"
    a: "yes"
"#,
        );

        assert_eq!(
            config.validate_configured_fields(),
            Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string()
            )
        );
    }
}
