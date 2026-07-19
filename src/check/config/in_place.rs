//! Applies only the additional `canon check --in-place` config restrictions.
//!
//! Expansion and general config validation run before this wrapper is
//! constructed. For ask, expansion also rejects prohibited configured xpec
//! fields before consuming them to build the runtime xpec. Git-backed commands
//! use their validated config directly; in-place commands use this module for
//! the separate mode contract.

use crate::config_types::{CheckConfig, Expectation};

#[derive(Clone)]
pub(crate) struct InPlaceCheckConfig {
    config: CheckConfig,
}

impl InPlaceCheckConfig {
    pub(crate) fn from_config(config: CheckConfig) -> InPlaceCheckConfig {
        InPlaceCheckConfig { config }
    }

    pub(crate) fn config(&self) -> &CheckConfig {
        &self.config
    }

    pub(crate) fn into_config(self) -> CheckConfig {
        self.config
    }

    pub(crate) fn validate_configured_fields(&self) -> Result<(), String> {
        // [uf,I4] Cached Result and its optional `cooldown` field apply to the
        // expectation-and-Git-state domain. The separate in-place contract has
        // no Git state and prohibits Git-backed fields throughout the config,
        // including on unselected expectations.
        // [T] Configuration provenance survives expansion, so explicit null
        // is rejected just like every other configured ignore value.
        if self.config.agent.ignore_configured {
            return Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string(),
            );
        }
        validate_in_place_expectations(&self.config.expectations)
    }
}

pub(super) fn validate_in_place_expectations(expectations: &[Expectation]) -> Result<(), String> {
    for (index, expectation) in expectations.iter().enumerate() {
        let incompatible_fields = expectation.in_place_compatibility.incompatible_fields();
        if !incompatible_fields.is_empty() {
            return Err(format!(
                "expectation {} is invalid in in-place mode: {}",
                index + 1,
                incompatible_fields
                    .iter()
                    .map(|field| format!(
                        "`{}` requires Git-backed check state",
                        field.config_name()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::config::expansion::{
        expand_raw_check_config_for_command, CheckConfigExpansionOptions,
    };
    use crate::config_types::{Cooldown, RawCheckConfig};

    fn parse_in_place_config(yaml: &str) -> InPlaceCheckConfig {
        let raw: RawCheckConfig = serde_saphyr::from_str(yaml).expect("parse raw config");
        let config = expand_raw_check_config_for_command(
            raw,
            CheckConfigExpansionOptions {
                in_place: true,
                ..CheckConfigExpansionOptions::default()
            },
        )
        .expect("expand raw config");
        InPlaceCheckConfig::from_config(config)
    }

    #[test] // xpec: 1r,I4,T
    fn ask_validation_precedes_runtime_expectation_replacement() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
presets:
  default: {}
  git-backed:
    diff-from: :against-tree
expectations:
  - q: "Configured Git-backed check"
    a: "yes"
    preset: git-backed
"#,
        )
        .expect("parse raw ask config");
        let error = expand_raw_check_config_for_command(
            raw,
            CheckConfigExpansionOptions {
                ask_question: Some("Does this work in place?"),
                in_place: true,
                ..CheckConfigExpansionOptions::default()
            },
        )
        .expect_err("configured prohibitions must fail before ask replacement");

        assert_eq!(
            error,
            "expectation 1 is invalid in in-place mode: \
             `diff-from` requires Git-backed check state"
        );
    }

    #[test] // xpec: I4
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

    #[test] // xpec: cg,eS
    fn target_values_have_identical_in_place_compatibility() {
        let config_with_target = |target: &str| {
            parse_in_place_config(&format!(
                r#"
presets:
  default: {{}}
expectations:
  - q: "Target compatibility"
    a: "yes"
    target: {target}
"#
            ))
        };
        let project = config_with_target("project");
        let diff = config_with_target("diff");

        assert_eq!(
            project.config().expectations[0].in_place_compatibility,
            diff.config().expectations[0].in_place_compatibility
        );
        let expected_error = Err("expectation 1 is invalid in in-place mode: \
             `target` requires Git-backed check state"
            .to_string());
        assert_eq!(project.validate_configured_fields(), expected_error);
        assert_eq!(
            diff.validate_configured_fields(),
            Err("expectation 1 is invalid in in-place mode: \
                 `target` requires Git-backed check state"
                .to_string(),)
        );
    }

    #[test] // xpec: uf,I4
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
            config.config().expectations[0].cooldown,
            Some(Cooldown {
                seconds: 7 * 24 * 60 * 60
            })
        );
        assert_eq!(
            config.validate_configured_fields(),
            Err("expectation 1 is invalid in in-place mode: \
                 `cooldown` requires Git-backed check state"
                .to_string())
        );
    }

    #[test] // xpec: I4
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

    #[test] // xpec: 1r,I4
    fn explicitly_empty_ignore_is_still_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default:
    ignore: []
expectations:
  - q: "In-place compatible"
    a: "yes"
"#,
        );

        assert!(config
            .config()
            .agent
            .ignore
            .as_ref()
            .is_some_and(Vec::is_empty));
        assert_eq!(
            config.validate_configured_fields(),
            Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string()
            )
        );
    }

    #[test] // xpec: cg,T
    fn explicit_null_git_backed_fields_are_still_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default: {}
expectations:
  - q: "Null does not erase field presence"
    a: "yes"
    diff-from: null
    target: null
    cooldown: null
    ignore: null
"#,
        );

        assert_eq!(
            config.validate_configured_fields(),
            Err("expectation 1 is invalid in in-place mode: \
                 `diff-from` requires Git-backed check state, \
                 `target` requires Git-backed check state, \
                 `cooldown` requires Git-backed check state, \
                 `ignore` requires Git-backed check state"
                .to_string())
        );
    }

    #[test] // xpec: cg,T
    fn explicit_null_default_ignore_is_still_invalid_in_place() {
        let config = parse_in_place_config(
            r#"
presets:
  default:
    ignore: null
expectations:
  - q: "Null does not enable path hiding"
    a: "yes"
"#,
        );

        assert_eq!(config.config().agent.ignore, None);
        assert_eq!(
            config.validate_configured_fields(),
            Err(
                "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                    .to_string()
            )
        );
    }
}
