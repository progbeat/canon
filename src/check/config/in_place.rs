use crate::config_types::{AgentConfig, CheckConfig, Expectation};

// In-place compatibility is intentionally separate from general config
// validation. Git-backed checks accept fields such as `cooldown`; only this
// mode-specific boundary rejects fields that require Git, cache, or hiding.
pub(crate) fn validate_in_place_global_config(config_agent: &AgentConfig) -> Result<(), String> {
    if config_agent.ignore.is_empty() {
        return Ok(());
    }
    Err("configured Git-backed-only ignore invalid in in-place mode".to_string())
}

pub(crate) fn validate_in_place_check_config(config: &CheckConfig) -> Result<(), String> {
    validate_in_place_global_config(&config.agent)?;
    validate_in_place_expectation_config(&config.agent, &config.expectations)
}

fn validate_in_place_expectation_config(
    config_agent: &AgentConfig,
    expectations: &[Expectation],
) -> Result<(), String> {
    // [Df] The configured-field ban is config-wide. Validate every expanded
    // xpec before selectors can hide mode-invalid configuration.
    for (index, expectation) in expectations.iter().enumerate() {
        let incompatibilities = in_place_incompatibilities(config_agent, expectation);
        if !incompatibilities.is_empty() {
            return Err(format!(
                "expectation {} is invalid in in-place mode: {}",
                index + 1,
                incompatibilities
                    .into_iter()
                    .map(InPlaceIncompatibility::message)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}

fn in_place_incompatibilities(
    config_agent: &AgentConfig,
    expectation: &Expectation,
) -> Vec<InPlaceIncompatibility> {
    [
        (
            expectation.diff_from.is_some(),
            InPlaceIncompatibility::DiffFrom,
        ),
        (expectation.target.is_some(), InPlaceIncompatibility::Target),
        (
            expectation.cooldown.is_some(),
            InPlaceIncompatibility::Cooldown,
        ),
        (
            !config_agent.ignore.is_empty() || !expectation.agent.ignore.is_empty(),
            InPlaceIncompatibility::Ignore,
        ),
    ]
    .into_iter()
    .filter_map(|(configured, incompatibility)| configured.then_some(incompatibility))
    .collect()
}

#[derive(Clone, Copy)]
enum InPlaceIncompatibility {
    DiffFrom,
    Target,
    Cooldown,
    Ignore,
}

impl InPlaceIncompatibility {
    fn message(self) -> &'static str {
        match self {
            Self::DiffFrom => "`diff-from` requires Git tree state",
            Self::Target => "`target` requires Git-backed evaluation context",
            // [jz,Df] `cooldown` is a supported expectation field for the
            // Git-backed checks whose cached-result policy it configures.
            // In-place is the exceptional mode: it has no cache state.
            Self::Cooldown => {
                "`cooldown` is supported by Git-backed checks but requires cache state"
            }
            Self::Ignore => "`ignore` requires Git-backed path hiding",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_in_place_check_config;
    use crate::config_types::{
        AgentConfig, CheckConfig, Cooldown, Expectation, ExpectationTarget, ExpectationTo,
        DEFAULT_DIFF_FROM,
    };

    #[test] // xpec: Df
    fn rejects_git_backed_only_config_after_expansion() {
        let agent = AgentConfig::default();
        let mut item = expectation(&agent, None);
        item.diff_from = Some(DEFAULT_DIFF_FROM.to_string());
        assert!(validate_in_place_check_config(&config_with(&agent, item)).is_err());

        let item = expectation(&agent, Some(ExpectationTarget::Diff));
        assert!(validate_in_place_check_config(&config_with(&agent, item)).is_err());

        let mut item = expectation(&agent, None);
        item.cooldown = Some(Cooldown {
            seconds: 24 * 60 * 60,
        });
        let error = validate_in_place_check_config(&config_with(&agent, item)).unwrap_err();
        assert!(
            error.contains("`cooldown` is supported by Git-backed checks but requires cache state")
        );

        let mut check_config = config_with(&agent, expectation(&agent, None));
        check_config.agent.ignore = vec!["target".to_string()];
        assert!(validate_in_place_check_config(&check_config).is_err());

        let mut item = expectation(&agent, None);
        item.agent.ignore = vec!["target".to_string()];
        assert!(validate_in_place_check_config(&config_with(&agent, item)).is_err());
    }

    fn expectation(agent: &AgentConfig, target: Option<ExpectationTarget>) -> Expectation {
        Expectation {
            to: ExpectationTo::Agent,
            rank: 0,
            q: "Does this behavior work?".to_string(),
            a: "yes".to_string(),
            question_context: String::new(),
            diff_from: None,
            target,
            question_answer_only: false,
            agent: agent.clone(),
            cooldown: None,
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
