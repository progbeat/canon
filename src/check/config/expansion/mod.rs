mod presets;
mod rank;
mod resolve;
mod source;

#[cfg(test)]
pub(crate) use resolve::expand_raw_check_config_with_options;
pub(crate) use resolve::{expand_raw_check_config_for_command, CheckConfigExpansionOptions};
pub(crate) use source::CheckConfigSource;

#[cfg(test)]
mod tests {
    use super::{
        expand_raw_check_config_with_options, resolve::expand_raw_check_config,
        CheckConfigExpansionOptions,
    };
    use crate::config_types::{
        Cooldown, ExpectationTarget, ExpectationTo, InPlaceIncompatibleField, RawCheckConfig,
        DEFAULT_DIFF_FROM,
    };

    #[test] // xpec: R5,3Z,LA,IJ,k4
    fn current_fields_resolve_scalar_values_addressee_and_rank() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
presets:
  default:
    to: caller
    rank: -2
    a: true
  shell: {}
expectations:
  - q: "7"
  - to: shell
    preset: shell
    q: "exit 0"
    rank: +3
"#,
        )
        .expect("parse current check config");

        let config = expand_raw_check_config(raw).expect("resolve current expectation fields");

        assert_eq!(config.expectations.len(), 2);
        assert_eq!(config.expectations[0].q, "7");
        assert_eq!(config.expectations[0].a, "true");
        assert_eq!(config.expectations[0].to, ExpectationTo::Caller);
        assert_eq!(config.expectations[0].rank, -2);
        assert_eq!(config.expectations[1].a, "0");
        assert_eq!(config.expectations[1].to, ExpectationTo::Shell);
        assert_eq!(config.expectations[1].rank, 3);
    }

    #[test] // xpec: R5
    fn non_string_questions_are_rejected_for_items_and_preset_defaults() {
        for yaml in [
            "presets:\n  default: {}\nxpecs:\n  - q: 7\n    a: yes\n",
            "presets:\n  default: {}\nxpecs:\n  - q: null\n    a: yes\n",
            "presets:\n  default:\n    q: 7\n    a: yes\nxpecs:\n  - {}\n",
            "presets:\n  default:\n    q: null\n    a: yes\nxpecs:\n  - {}\n",
        ] {
            assert!(serde_saphyr::from_str::<RawCheckConfig>(yaml).is_err());
        }
    }

    #[test] // xpec: R5
    fn explicit_null_answer_resolves_to_string() {
        let raw: RawCheckConfig =
            serde_saphyr::from_str("presets:\n  default:\n    a: null\nxpecs:\n  - q: question\n")
                .expect("parse explicit null answer");

        let config = expand_raw_check_config(raw).expect("resolve explicit null answer");

        assert_eq!(config.expectations[0].a, "null");
    }

    #[test] // xpec: eS
    fn unsupported_expectation_target_is_rejected_during_expansion() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - q: "Does it pass?"
    a: "yes"
    target: whole-project
"#,
        )
        .expect("parse raw check config");

        let error = expand_raw_check_config(raw).unwrap_err();

        assert_eq!(
            error,
            "expectation 1 target: unsupported target: whole-project"
        );
    }

    #[test] // xpec: eS
    fn explicit_project_target_is_supported() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default: {}
expectations:
  - q: "Does it pass?"
    a: "yes"
    target: project
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config(raw).expect("expand config");

        assert_eq!(
            config.expectations[0].target,
            Some(ExpectationTarget::Project)
        );
    }

    #[test] // xpec: LA
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

    #[test] // xpec: LA
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

    #[test] // xpec: 21
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
  right:
    q: "Right question"
    a: "right"
    rank: 7
    models: ["right-model"]
expectations:
  - q: "Item question"
    preset: left+right
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
        assert_eq!(expectation.to, ExpectationTo::Agent);
        assert_eq!(expectation.diff_from, DEFAULT_DIFF_FROM);
    }

    #[test] // xpec: 21,T
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
    models: ["preset-model"]
    thinking: high
    ignore: ["tmp/**"]
    plugins: ["preset-plugin@marketplace"]
  right:
    instructions: null
    to: null
    rank: null
    diff-from: null
    target: null
    cooldown: null
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
    to: null
    rank: null
    diff-from: null
    target: null
    cooldown: null
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
            assert_eq!(expectation.to, ExpectationTo::Agent);
            assert_eq!(expectation.rank, 0);
            assert_eq!(expectation.diff_from, DEFAULT_DIFF_FROM);
            assert_eq!(expectation.target, None);
            assert_eq!(expectation.cooldown, None);
            assert!(expectation.agent.models.is_empty());
            assert_eq!(expectation.agent.thinking, "low");
            assert_eq!(expectation.agent.ignore, None);
            assert!(expectation.agent.plugins.is_empty());
            assert_eq!(
                expectation.in_place_compatibility.incompatible_fields(),
                &[
                    InPlaceIncompatibleField::DiffFrom,
                    InPlaceIncompatibleField::Target,
                    InPlaceIncompatibleField::Cooldown,
                    InPlaceIncompatibleField::Ignore,
                ]
            );
        }
    }

    #[test] // xpec: nK,LA
    fn default_agent_preset_option_only_changes_config_agent() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
version: 1
presets:
  default:
    models: ["default-model"]
  smart:
    models: ["smart-model"]
expectations:
  - q: "Does the default expectation preset stay default?"
    a: "yes"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config_with_options(
            raw,
            CheckConfigExpansionOptions {
                default_agent_preset: Some("smart"),
                ask_question: None,
                in_place: false,
            },
        )
        .expect("expand config");

        assert_eq!(config.agent.models, vec!["smart-model".to_string()]);
        assert_eq!(
            config.expectations[0].agent.models,
            vec!["default-model".to_string()]
        );
    }

    #[test] // xpec: Ky,nK,LA
    fn ask_question_uses_selected_preset_defaults_and_explicit_ask_fields() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
presets:
  default: {}
  smart:
    q: "Preset question"
    a: "yes"
    to: caller
    rank: 9
    instructions: "Use selected preset context."
    diff-from: HEAD~1
    target: diff
    models: ["smart-model"]
expectations:
  - q: "Configured check expectation"
    a: "no"
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config_with_options(
            raw,
            CheckConfigExpansionOptions {
                default_agent_preset: Some("smart"),
                ask_question: Some("Does preset ask work?"),
                in_place: false,
            },
        )
        .expect("expand ask config");

        assert_eq!(config.expectations.len(), 1);
        let expectation = &config.expectations[0];
        assert_eq!(expectation.q, "Does preset ask work?");
        assert!(expectation.a.is_empty());
        assert_eq!(expectation.to, ExpectationTo::Agent);
        assert_eq!(expectation.rank, 9);
        assert_eq!(expectation.question_context, "Use selected preset context.");
        assert_eq!(expectation.diff_from, "HEAD~1");
        assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
        assert_eq!(expectation.agent.models, vec!["smart-model".to_string()]);
    }

    // xpec: LA

    #[test]
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
        // xpec: 1r,LA
        assert_eq!(
            expectation.cooldown,
            Some(Cooldown {
                seconds: 7 * 24 * 60 * 60
            })
        );
        assert_eq!(expectation.agent.models, vec!["preset-model".to_string()]);
        assert_eq!(expectation.agent.thinking, "high");
    }

    #[test] // xpec: 3Z
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

    #[test] // xpec: LA,3Z
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

    #[test] // xpec: LA
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

    #[test] // xpec: LA,Ijn
    fn question_answer_only_uses_resolved_preset_defaults() {
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
        assert!(!expectation.question_answer_only);
        assert_eq!(expectation.question_context, "Use the preset instructions.");
        assert_eq!(expectation.diff_from, "master");
        assert_eq!(expectation.target, Some(ExpectationTarget::Diff));
        assert_eq!(expectation.agent.thinking, "high");
    }

    #[test] // xpec: LA
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
        // xpec: 1r,LA
        assert_eq!(
            expectation.cooldown,
            Some(Cooldown {
                seconds: 24 * 60 * 60
            })
        );
        assert_eq!(expectation.agent.thinking, "high");
    }
}
