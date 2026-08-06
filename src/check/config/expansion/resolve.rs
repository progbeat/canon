//! Resolves raw check configuration into fully expanded runtime configuration.

use super::presets::{raw_presets_from_config, resolve_preset_closure, resolve_presets};
use crate::check::config::in_place::validate_raw_in_place_expectations;
use crate::config_types::{CheckConfig, RawCheckConfig};

mod defaults;
mod fields;
mod item;

use fields::raw_canonical_ask_xpec;
use item::RawExpectationExpansion;

#[derive(Default)]
pub(crate) struct CheckConfigExpansionOptions<'a> {
    pub(crate) default_agent_preset: Option<&'a str>,
    pub(crate) ask_question: Option<&'a str>,
    pub(crate) in_place: bool,
}

pub(crate) fn expand_raw_check_config_for_command(
    raw: RawCheckConfig,
    options: CheckConfigExpansionOptions<'_>,
) -> Result<CheckConfig, String> {
    let RawCheckConfig {
        version,
        presets,
        agent,
        expectations: configured_expectations,
    } = raw;
    // Raw expansion is the only layer that consumes preset names. Command
    // execution receives the returned `CheckConfig`, which carries resolved
    // agent/expectation fields and no preset map to inspect later.
    let raw_presets = raw_presets_from_config(presets, agent)?;
    let default_agent_preset = options.default_agent_preset.unwrap_or("default");
    let runtime_expectations;
    let resolved_presets = match options.ask_question {
        Some(question) => {
            // [l] Configured check xpecs are not inputs to the canonical ask
            // xpec. In-place mode follows their raw preset references solely
            // to validate configured Git-dependent field presence; it does not
            // expand discarded questions, answers, or other values.
            if options.in_place {
                validate_raw_in_place_expectations(&configured_expectations, &raw_presets)?;
            }
            let resolved_presets = resolve_preset_closure(&raw_presets, default_agent_preset)?;
            let expansion = RawExpectationExpansion {
                presets: &resolved_presets,
            };
            runtime_expectations = expansion
                .expand_items(vec![raw_canonical_ask_xpec(question, default_agent_preset)])?;
            resolved_presets
        }
        None => {
            let resolved_presets = resolve_presets(raw_presets)?;
            let expansion = RawExpectationExpansion {
                presets: &resolved_presets,
            };
            runtime_expectations = expansion.expand_items(configured_expectations)?;
            resolved_presets
        }
    };
    let resolved_default_agent_preset = resolved_presets
        .get(default_agent_preset)
        .ok_or_else(|| format!("unknown preset: {}", default_agent_preset))?;
    let default_agent = resolved_default_agent_preset.agent_config();
    // `canon ask` supplies its canonical `Xpec(to=AGENT, q=question, a='')`
    // through the same typed item expansion used by check. Its explicit fields
    // keep precedence and the selected preset supplies only omitted fields.
    Ok(CheckConfig {
        version,
        agent: default_agent,
        expectations: runtime_expectations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{ExpectationTarget, ExpectationTo};

    #[test] // xpec: nK,1H
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

        let config = expand_raw_check_config_for_command(
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

    #[test] // xpec: l,nK,1H
    fn canonical_ask_xpec_keeps_explicit_fields_and_resolves_preset_defaults() {
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

        let config = expand_raw_check_config_for_command(
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

    #[test] // xpec: l
    fn ask_does_not_expand_discarded_configured_expectations_or_unused_presets() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
presets:
  default: {}
  unused:
    preset: missing-parent
expectations:
  - preset: missing-expectation-preset
    target: unsupported
    cooldown: not-a-duration
    q-scope: unsupported
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config_for_command(
            raw,
            CheckConfigExpansionOptions {
                ask_question: Some("Does ask ignore discarded check xpecs?"),
                ..CheckConfigExpansionOptions::default()
            },
        )
        .expect("expand ask config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            "Does ask ignore discarded check xpecs?"
        );
        assert!(config.expectations[0].a.is_empty());
        assert_eq!(config.expectations[0].to, ExpectationTo::Agent);
    }

    #[test] // xpec: l
    fn in_place_ask_ignores_unknown_preset_on_discarded_compatible_expectation() {
        let raw: RawCheckConfig = serde_saphyr::from_str(
            r#"
presets:
  default: {}
expectations:
  - preset: missing-expectation-preset
"#,
        )
        .expect("parse raw check config");

        let config = expand_raw_check_config_for_command(
            raw,
            CheckConfigExpansionOptions {
                ask_question: Some("Does in-place ask ignore discarded check xpecs?"),
                in_place: true,
                ..CheckConfigExpansionOptions::default()
            },
        )
        .expect("expand in-place ask config");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].q,
            "Does in-place ask ignore discarded check xpecs?"
        );
        assert!(config.expectations[0].a.is_empty());
        assert_eq!(config.expectations[0].to, ExpectationTo::Agent);
    }
}
