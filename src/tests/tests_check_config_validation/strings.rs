use crate::check_selection::parse_cooldown;
use crate::tests::parse_check_config;

#[test]
fn check_config_rejects_blank_agent_ignore_pattern() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "   "
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
}

#[test]
fn check_config_rejects_cooldown_with_surrounding_whitespace() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
    cooldown: " 1d "
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: malformed
"#
    )
    .is_err());
}

#[test]
fn cooldown_parser_rejects_non_ascii_unit_without_panicking() {
    assert!(parse_cooldown("1д").is_err());
}

#[test]
fn check_config_rejects_missing_required_fields() {
    assert!(parse_check_config("version: 1\n").is_err());
    assert!(parse_check_config("version: 1\nagent: {}\nexpectations: []\n").is_err());
    assert!(parse_check_config(
        "version: 1\nagent:\n  instructions: x\n  ignore: []\nexpectations:\n  - q: x\n    a: y\n"
    )
    .is_err());
}

#[test]
fn check_config_rejects_blank_agent_instructions() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "   "
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\u200B"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\uFE0F"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\uFFF9"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\U00013430"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\u2800"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
}

#[test]
fn check_config_rejects_visually_blank_expectation_questions() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "\u200B"
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "\u180B"
    a: y
"#
    )
    .is_err());
}

#[test]
fn check_config_accepts_free_form_expected_answers() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: maybe
"#
    )
    .is_ok());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: Rust
"#
    )
    .is_ok());
}

#[test]
fn check_config_rejects_reserved_or_blank_expected_answers() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: idk
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: malformed
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: "   "
"#
    )
    .is_err());
}
