mod agent;
mod cooldown;
mod expectation;

pub(crate) use agent::{
    check_config_loads_plugins, normalize_agent_ignore_pattern_for_config,
    validate_resolved_agent_config,
};
pub(crate) use cooldown::parse_cooldown_config;
pub(crate) use expectation::{validate_ask_config, validate_check_config};

#[cfg(test)]
mod tests {
    use super::parse_cooldown_config;
    use super::validate_check_config;
    use crate::check::core::ERROR_SCOPE_TOO_NARROW;
    use crate::config_types::{
        AgentConfig, CheckConfig, Cooldown, CooldownConfig, Expectation, ExpectationTarget,
        DEFAULT_DIFF_FROM,
    };

    #[test] // xpec: m
    fn cooldown_config_accepts_compact_positive_duration() {
        assert_eq!(
            parse_cooldown_config(&CooldownConfig("30m".to_string())).unwrap(),
            Cooldown { seconds: 30 * 60 }
        );
    }

    #[test] // xpec: m
    fn cooldown_config_rejects_mapping_and_fail_specific_forms() {
        assert!(serde_saphyr::from_str::<CooldownConfig>("fail: 1h").is_err());
        assert!(serde_saphyr::from_str::<CooldownConfig>("pass: 7d").is_err());
    }

    #[test] // xpec: a,kK
    fn invalid_expected_answer_error_uses_expectation_block_format() {
        let question = "What\nlanguage?";
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: vec![Expectation {
                to: crate::config_types::ExpectationTo::Agent,
                rank: 0,
                q: question.to_string(),
                a: "Rust\t".to_string(),
                question_context: String::new(),
                diff_from: DEFAULT_DIFF_FROM.to_string(),
                target: None,
                agent,
                cooldown: None,
                q_scope: Default::default(),
                in_place_compatibility: Default::default(),
            }],
        };

        let error = validate_check_config(&config).unwrap_err();

        let mut lines = error.lines();
        let header = lines.next().unwrap();
        assert!(header.ends_with(". ERROR"));
        assert_ne!(header, ". ERROR");
        assert_eq!(lines.next(), Some("What\\nlanguage?"));
        assert_eq!(lines.next(), Some("Error: invalid-expected-answer"));
        assert_eq!(
            lines.next(),
            Some("Evidence: configured expected answer `Rust\\\\t` does not match answer pattern ^[-_a-z0-9]+$")
        );
        assert_eq!(lines.next(), None);
    }

    #[test] // xpec: T5
    fn evaluator_error_token_uses_specific_validation_path() {
        let agent = AgentConfig::default();
        let mut item = expectation(&agent, None);
        item.a = ERROR_SCOPE_TOO_NARROW.to_string();

        let error = validate_check_config(&config_with(&agent, item)).unwrap_err();

        assert!(
            error.contains("Error: expected answer must not be an evaluator error token"),
            "{error}"
        );
    }

    #[test] // xpec: MH
    fn duplicate_expectation_ids_are_rejected_even_when_targets_differ() {
        let agent = AgentConfig::default();
        let expectation = |target| Expectation {
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            q: "Does this behavior work?".to_string(),
            a: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target,
            agent: agent.clone(),
            cooldown: None,
            q_scope: Default::default(),
            in_place_compatibility: Default::default(),
        };
        let config = CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: vec![
                expectation(None),
                expectation(Some(ExpectationTarget::Project)),
                expectation(Some(ExpectationTarget::Diff)),
            ],
        };

        let error = validate_check_config(&config).unwrap_err();

        assert!(error.starts_with("duplicate expectation ID: "), "{error}");
    }

    #[test] // xpec: MH
    fn empty_expectation_sequence_is_valid() {
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            agent,
            expectations: Vec::new(),
        };

        assert!(validate_check_config(&config).is_ok());
    }

    #[test] // xpec: MH
    fn question_strings_have_no_extra_content_restriction() {
        let agent = AgentConfig::default();
        for question in ["", " \t\n"] {
            let mut item = expectation(&agent, None);
            item.q = question.to_string();

            assert!(validate_check_config(&config_with(&agent, item)).is_ok());
        }
    }

    #[test] // xpec: m
    fn git_backed_config_accepts_canonical_cooldown() {
        let agent = AgentConfig::default();
        let mut item = expectation(&agent, None);
        item.cooldown = Some(Cooldown {
            seconds: 24 * 60 * 60,
        });

        assert!(
            validate_check_config(&config_with(&agent, item)).is_ok(),
            "a canonical cooldown must be valid in Git-backed check config"
        );
    }

    fn expectation(agent: &AgentConfig, target: Option<ExpectationTarget>) -> Expectation {
        Expectation {
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            q: "Does this behavior work?".to_string(),
            a: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target,
            agent: agent.clone(),
            cooldown: None,
            q_scope: Default::default(),
            in_place_compatibility: Default::default(),
        }
    }

    fn config_with(agent: &AgentConfig, expectation: Expectation) -> CheckConfig {
        CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: vec![expectation],
        }
    }
}
