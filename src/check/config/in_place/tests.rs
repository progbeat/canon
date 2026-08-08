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

#[test] // xpec: 1r,90,T5
fn raw_ask_validation_precedes_canonical_xpec_expansion() {
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
    .expect_err("configured prohibitions must fail before canonical ask expansion");

    assert_eq!(
        error,
        "expectation 1 is invalid in in-place mode: \
             `diff-from` requires Git-backed check state"
    );
}

#[test] // xpec: 90
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

#[test] // xpec: 90
fn target_values_are_equally_invalid_in_place() {
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

#[test] // xpec: m,90
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

#[test] // xpec: 90
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

#[test] // xpec: 1r,90
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

    assert_eq!(
        config.validate_configured_fields(),
        Err(
            "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                .to_string()
        )
    );
}

#[test] // xpec: 90,T5
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

#[test] // xpec: 90,T5
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

    assert_eq!(
        config.validate_configured_fields(),
        Err(
            "configured `ignore` is invalid in in-place mode because path hiding requires Git"
                .to_string()
        )
    );
}
