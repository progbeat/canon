use super::*;

#[test] // xpec: 1H
fn legacy_agent_config_still_expands_to_default_preset() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
agent:
  model:
    primary: "legacy-primary"
    fallbacks: ["legacy-fallback"]
  thinking: high
  ignore: ["tmp/**"]
expectations:
  - q: "Does the legacy agent expand?"
    a: "yes"
"#,
    )
    .expect("parse legacy raw check config");

    let config = expand_raw_check_config(raw).expect("expand legacy config");

    assert_eq!(
        config.agent.models,
        vec!["legacy-primary".to_string(), "legacy-fallback".to_string()]
    );
    assert_eq!(config.agent.thinking, "high");
    assert_eq!(config.agent.ignore, Some(vec!["tmp/**".to_string()]));
}

#[test] // xpec: 1H
fn preset_inherits_from_named_preset_with_preset_key() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    models: ["default-model"]
    thinking: medium
    ignore: ["tmp/**"]
  smart:
    preset: default
    thinking: high
expectations:
  - q: "Does the smart preset inherit?"
    a: "yes"
    preset: smart
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    let expectation = &config.expectations[0];
    assert_eq!(expectation.agent.models, vec!["default-model".to_string()]);
    assert_eq!(expectation.agent.thinking, "high");
    assert_eq!(expectation.agent.ignore, Some(vec!["tmp/**".to_string()]));
}

#[test] // xpec: 1H
fn selected_presets_resolve_item_then_right_to_left_then_implementation_defaults() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    to: caller
    diff-from: default-branch
  left:
    q: "Left question"
    a: "left"
    instructions: "Left instructions."
    models: ["left-model"]
    thinking: high
    q-scope: ["src"]
  right:
    q: "Right question"
    a: "right"
    rank: 7
    models: ["right-model"]
expectations:
  - q: "Item question"
    preset: left + right
    models: ["item-model"]
"#,
    )
    .expect("parse multi-preset check config");

    let config = expand_raw_check_config(raw).expect("expand multi-preset config");

    let expectation = &config.expectations[0];
    assert_eq!(expectation.q, "Item question");
    assert_eq!(expectation.a, "right");
    assert_eq!(expectation.rank, 7);
    assert_eq!(expectation.question_context, "Left instructions.");
    assert_eq!(expectation.agent.models, vec!["item-model".to_string()]);
    assert_eq!(expectation.agent.thinking, "high");
    assert_eq!(expectation.q_scope, QScope::Paths(vec!["src".to_string()]));
    assert_eq!(expectation.to, ExpectationTo::Agent);
    assert_eq!(expectation.diff_from, DEFAULT_DIFF_FROM);
}

#[test] // xpec: 1H,T5
fn explicit_null_blocks_lower_precedence_optional_field_values() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
presets:
  default: {}
  left:
    instructions: "Preset context"
    to: caller
    rank: 7
    diff-from: main
    target: diff
    cooldown: 7d
    q-scope: ["src"]
    models: ["preset-model"]
    thinking: high
    ignore: ["tmp/**"]
    plugins: ["preset-plugin@marketplace"]
  right:
    instructions: null
    rank: null
    diff-from: null
    target: null
    cooldown: null
    q-scope: null
    models: null
    thinking: null
    ignore: null
    plugins: null
expectations:
  - q: "Does preset null win?"
    a: "yes"
    preset: left+right
  - q: "Does item null win?"
    a: "yes"
    preset: left
    instructions: null
    rank: null
    diff-from: null
    target: null
    cooldown: null
    q-scope: null
    models: null
    thinking: null
    ignore: null
    plugins: null
"#,
    )
    .expect("parse explicit null preset values");

    let config = expand_raw_check_config(raw).expect("expand explicit null preset values");
    for expectation in &config.expectations {
        assert!(expectation.question_context.is_empty());
        assert_eq!(expectation.to, ExpectationTo::Caller);
        assert_eq!(expectation.rank, 0);
        assert_eq!(expectation.diff_from, DEFAULT_DIFF_FROM);
        assert_eq!(expectation.target, None);
        assert_eq!(expectation.cooldown, None);
        assert_eq!(expectation.q_scope, QScope::Auto);
        assert!(expectation.agent.models.is_empty());
        assert_eq!(expectation.agent.thinking, "low");
        assert_eq!(expectation.agent.ignore, None);
        assert!(expectation.agent.plugins.is_empty());
    }
}
