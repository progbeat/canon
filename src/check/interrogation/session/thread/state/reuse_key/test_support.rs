use crate::config_types::AgentConfig;
use crate::evaluator::{
    evaluator_thread_config_identity, EvaluatorThreadConfigIdentity,
    EvaluatorThreadConfigIdentityContext,
};

pub(super) fn make_evaluator_thread_config_identity(
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
