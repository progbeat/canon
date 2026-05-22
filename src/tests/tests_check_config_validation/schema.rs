use crate::tests::{check_config_yaml, parse_check_config};

#[test]
fn check_config_accepts_minimal_schema() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    assert_eq!(config.expectations.len(), 2);
    assert_eq!(config.agent.model.primary.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(config.agent.model.fallbacks, vec!["gpt-5.3-codex-spark"]);
    assert_eq!(config.agent.thinking, "medium");
    assert_eq!(config.agent.instructions, None);
    assert_eq!(config.agent.ignore, vec!["target/**"]);
}

#[test]
fn check_config_defaults_thinking_to_low() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#,
    )
    .unwrap();
    assert_eq!(config.agent.thinking, "low");
    assert!(parse_check_config(
        r#"
version: 1
agent:
  thinking: unsupported
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(&format!(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: "yes{}no"
"#,
        '\u{2028}'
    ))
    .is_err());
}

#[test]
fn check_config_accepts_agent_ignore_wildcards() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "logs/*"
  plugins: []
expectations:
  - q: x
    a: y
"#,
    )
    .unwrap();
    assert_eq!(config.agent.ignore, vec!["logs/*"]);
}

#[test]
fn check_config_allows_extra_expectation_fields() {
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - id: extra
    q: "Question?"
    a: "yes"
"#;
    let config = parse_check_config(yaml).unwrap();
    assert_eq!(config.expectations[0].q, "Question?");
    assert_eq!(config.expectations[0].a, "yes");
}
