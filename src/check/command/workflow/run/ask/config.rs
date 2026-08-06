use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, ExpectationTo, DEFAULT_DIFF_FROM,
};

pub(super) fn ask_query_config(
    config: Result<CheckConfig, String>,
    config_optional: bool,
    question: &str,
) -> Result<CheckConfig, String> {
    // `load_ask_config` has omitted configured check expectations and resolved
    // the one canonical ask xpec. Remaining errors from an explicitly selected
    // config source or ask preset are returned; only the command-default config
    // is optional and may fall back to implementation defaults.
    match config {
        Ok(config) => Ok(config),
        Err(err) if !config_optional => Err(err),
        Err(_) => Ok(ask_query_config_with_agent(
            question,
            AgentConfig::implementation_default(),
        )),
    }
}

fn ask_query_config_with_agent(question: &str, agent: AgentConfig) -> CheckConfig {
    CheckConfig {
        version: 1,
        agent: agent.clone(),
        expectations: vec![Expectation {
            to: ExpectationTo::Agent,
            q: question.to_string(),
            a: String::new(),
            rank: 0,
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent,
            cooldown: None,
            q_scope: Default::default(),
            in_place_compatibility: Default::default(),
        }],
    }
}
