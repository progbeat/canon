use super::*;

#[test] // xpec: 1H
fn preset_supplies_expectation_field_defaults() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    q: "Does the preset supply defaults?"
    a: "yes"
    instructions: "Use the preset instructions."
    diff-from: master
    target: diff
    cooldown: 7d
    models: ["preset-model"]
    thinking: high
expectations:
  - {}
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    let expectation = &config.expectations[0];
    assert_eq!(expectation.q, "Does the preset supply defaults?");
    assert_eq!(expectation.a, "yes");
    assert_eq!(expectation.question_context, "Use the preset instructions.");
    assert_eq!(expectation.diff_from, "master");
    assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
    // xpec: 1r,1H
    assert_eq!(
        expectation.cooldown,
        Some(Cooldown {
            seconds: 7 * 24 * 60 * 60
        })
    );
    assert_eq!(expectation.agent.models, vec!["preset-model".to_string()]);
    assert_eq!(expectation.agent.thinking, "high");
}

#[test] // xpec: MH
fn extra_xpec_fields_do_not_change_resolved_fields() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default: {}
expectations:
  - q: "Does the explicit item stay explicit?"
    a: "yes"
    glob: "specs/*.md"
    q_template: "Generated: {{ read(path) }}"
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(
        config.expectations[0].q,
        "Does the explicit item stay explicit?"
    );
    assert_eq!(config.expectations[0].a, "yes");
}

#[test] // xpec: 1H,MH
fn extra_preset_fields_do_not_override_xpec_fields() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    include: "expects/*.yml"
    glob: "specs/*.md"
    q_template: "Generated: {{ read(path) }}"
    q: "Does the preset question lose?"
    a: "no"
expectations:
  - q: "Does the explicit item stay explicit?"
    a: "yes"
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(
        config.expectations[0].q,
        "Does the explicit item stay explicit?"
    );
    assert_eq!(config.expectations[0].a, "yes");
}

#[test] // xpec: 1H
fn preset_supplies_missing_fields_for_declared_explicit_items() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    a: "yes"
expectations:
  - q: "Does the item question use the preset answer?"
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    assert_eq!(config.expectations.len(), 1);
    assert_eq!(
        config.expectations[0].q,
        "Does the item question use the preset answer?"
    );
    assert_eq!(config.expectations[0].a, "yes");
}

#[test] // xpec: 1H,Ijn
fn expectation_uses_resolved_preset_defaults() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    instructions: "Use the preset instructions."
    diff-from: master
    target: diff
    thinking: high
expectations:
  - q: "Does q matching keep preset context?"
    a: "yes"
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    let expectation = &config.expectations[0];
    assert_eq!(expectation.question_context, "Use the preset instructions.");
    assert_eq!(expectation.diff_from, "master");
    assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
    assert_eq!(expectation.agent.thinking, "high");
}

#[test] // xpec: 1H
fn expectation_fields_override_preset_defaults() {
    let raw: RawCheckConfig = serde_saphyr::from_str(
        r#"
version: 1
presets:
  default:
    q: "Does the preset lose?"
    a: "no"
    instructions: "Preset instructions."
    diff-from: master
    cooldown: 7d
    thinking: medium
expectations:
  - q: "Does the item win?"
    a: "yes"
    instructions: " Item instructions. "
    diff-from: " HEAD~1 "
    cooldown: 1d
    thinking: high
"#,
    )
    .expect("parse raw check config");

    let config = expand_raw_check_config(raw).expect("expand config");

    let expectation = &config.expectations[0];
    assert_eq!(expectation.q, "Does the item win?");
    assert_eq!(expectation.a, "yes");
    assert_eq!(expectation.question_context, " Item instructions. ");
    assert_eq!(expectation.diff_from, " HEAD~1 ");
    // xpec: 1r,1H
    assert_eq!(
        expectation.cooldown,
        Some(Cooldown {
            seconds: 24 * 60 * 60
        })
    );
    assert_eq!(expectation.agent.thinking, "high");
}
