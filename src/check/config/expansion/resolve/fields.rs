use crate::config_types::{
    ConfiguredValue, ExpectationTarget, QScope, QScopeConfig, RawExpectationCommonConfig,
    RawExpectationFields, RawExpectationItem, RawExpectationSettings, DEFAULT_DIFF_FROM,
};
use crate::scope::sanitize_scope;

pub(super) fn raw_canonical_ask_xpec(question: &str, preset: &str) -> RawExpectationItem {
    RawExpectationItem::Unresolved(RawExpectationFields {
        explicit_q: Some(question.to_string()),
        a: Some(String::new()),
        common: RawExpectationCommonConfig {
            to: ConfiguredValue::some(crate::config_types::ExpectationTo::Agent),
            settings: RawExpectationSettings {
                preset: Some(preset.to_string()),
                ..RawExpectationSettings::default()
            },
            ..RawExpectationCommonConfig::default()
        },
    })
}

pub(super) fn resolved_question_context(context: Option<String>) -> String {
    context.unwrap_or_default()
}

pub(super) fn resolved_expectation_diff_from(diff_from: Option<String>) -> String {
    diff_from.unwrap_or_else(|| DEFAULT_DIFF_FROM.to_string())
}

pub(super) fn resolve_expectation_target(
    target: Option<String>,
) -> Result<Option<ExpectationTarget>, String> {
    target.map(|target| target.parse()).transpose()
}

pub(super) fn resolve_q_scope(q_scope: Option<QScopeConfig>) -> Result<QScope, String> {
    match q_scope {
        None => Ok(QScope::Auto),
        Some(QScopeConfig::Mode(mode)) if mode == "auto" => Ok(QScope::Auto),
        Some(QScopeConfig::Mode(mode)) => Err(format!("unsupported q-scope mode: {}", mode)),
        Some(QScopeConfig::Paths(paths)) => sanitize_scope(&paths).map(QScope::Paths),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{expand_raw_check_config_for_command, CheckConfigExpansionOptions};
    use crate::check::config::validation::validate_check_config;
    use crate::config_types::{ExpectationTo, RawCheckConfig};

    fn expand_raw_check_config(
        raw: RawCheckConfig,
    ) -> Result<crate::config_types::CheckConfig, String> {
        expand_raw_check_config_for_command(raw, CheckConfigExpansionOptions::default())
    }

    #[test] // xpec: MH,1H,H9,Eg
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

    #[test] // xpec: MH
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

    #[test] // xpec: MH
    fn configured_to_must_name_an_addressee() {
        for yaml in [
            "presets:\n  default: {}\nxpecs:\n  - q: question\n    a: yes\n    to: null\n",
            "presets:\n  default:\n    to: null\nxpecs:\n  - q: question\n    a: yes\n",
        ] {
            assert!(serde_saphyr::from_str::<RawCheckConfig>(yaml).is_err());
        }
    }

    #[test] // xpec: MH
    fn explicit_null_answer_resolves_to_string() {
        let raw: RawCheckConfig =
            serde_saphyr::from_str("presets:\n  default:\n    a: null\nxpecs:\n  - q: question\n")
                .expect("parse explicit null answer");

        let config = expand_raw_check_config(raw).expect("resolve explicit null answer");

        assert_eq!(config.expectations[0].a, "null");
    }

    #[test] // xpec: MH,a
    fn scalar_stringification_precedes_answer_pattern_validation() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            "presets:\n  default: {}\nxpecs:\n  - q: question\n    a: 1.5\n",
        )
        .expect("parse scalar expected answer");

        let config = expand_raw_check_config(raw).expect("resolve scalar expected answer");

        assert_eq!(config.expectations[0].a, "1.5");
        assert!(validate_check_config(&config)
            .unwrap_err()
            .contains("does not match answer pattern"));
    }
}
