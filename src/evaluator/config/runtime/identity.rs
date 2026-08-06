//! Stable identity of a rendered evaluator runtime configuration.

use super::{effective_evaluator_thread_model, evaluator_reasoning_effort};
use crate::config_types::AgentConfig;
use std::collections::BTreeSet;

pub(crate) struct EvaluatorThreadConfigIdentityContext<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) dynamic_tools: &'a [serde_json::Value],
}

// This is the evaluator component's complete public identity for evaluator
// configuration that may vary among threads owned by one invocation-local
// runner. Callers combine this opaque identity with their own thread inputs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EvaluatorThreadConfigIdentity {
    model: Option<String>,
    reasoning_effort: Option<String>,
    plugins: BTreeSet<String>,
    dynamic_tools: Vec<serde_json::Value>,
}

pub(crate) fn evaluator_thread_config_identity(
    context: EvaluatorThreadConfigIdentityContext<'_>,
) -> EvaluatorThreadConfigIdentity {
    EvaluatorThreadConfigIdentity {
        model: effective_evaluator_thread_model(context.agent, context.model).map(str::to_string),
        reasoning_effort: evaluator_reasoning_effort(context.thinking).map(str::to_string),
        plugins: context.agent.plugins.iter().cloned().collect(),
        dynamic_tools: context.dynamic_tools.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_evaluator_thread_config_identity(
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        dynamic_tools: &[serde_json::Value],
    ) -> EvaluatorThreadConfigIdentity {
        evaluator_thread_config_identity(EvaluatorThreadConfigIdentityContext {
            agent,
            model,
            thinking,
            dynamic_tools,
        })
    }

    #[test] // xpec: gN,fD
    fn evaluator_thread_config_identity_separates_variable_compatibility_inputs() {
        let agent = AgentConfig {
            models: vec!["fallback-a".to_string()],
            plugins: vec!["plugin-a".to_string()],
            ..AgentConfig::default()
        };
        let dynamic_tools = vec![json!({"name": "tool-a"})];
        let baseline_config_identity =
            make_evaluator_thread_config_identity(&agent, None, "medium", &dynamic_tools);
        let mut changed_agent = agent.clone();

        assert_eq!(
            baseline_config_identity,
            make_evaluator_thread_config_identity(
                &agent,
                Some("fallback-a"),
                "medium",
                &dynamic_tools,
            )
        );
        changed_agent.models = vec!["fallback-b".to_string()];
        assert_ne!(
            baseline_config_identity,
            make_evaluator_thread_config_identity(&changed_agent, None, "medium", &dynamic_tools)
        );
        assert_ne!(
            baseline_config_identity,
            make_evaluator_thread_config_identity(&agent, None, "high", &dynamic_tools)
        );
        changed_agent = agent.clone();
        changed_agent.plugins = vec!["plugin-b".to_string()];
        assert_ne!(
            baseline_config_identity,
            make_evaluator_thread_config_identity(&changed_agent, None, "medium", &dynamic_tools)
        );
        assert_ne!(
            baseline_config_identity,
            make_evaluator_thread_config_identity(
                &agent,
                None,
                "medium",
                &[json!({"name": "tool-b"})],
            )
        );
    }
}
