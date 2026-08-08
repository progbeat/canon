// Interrogation owns evaluator interaction and response normalization. Policy,
// query, result, session, and state model separate stages of that lifecycle.
pub(super) mod policy;
pub(super) mod query;
mod result;
mod session;
pub(super) mod state;
mod turn_kind;

pub(crate) use result::{
    finalize_interrogation_answer, interrogation_result_from_answer, scope_narrowing_log_fields,
    write_expectation_result_event, write_query_result_event, write_query_review_required_event,
};
pub(crate) use session::{
    interrogate_with_model_fallbacks, InterrogationSession, ModelFallbackInterrogation,
    ModelFallbackOutput,
};
pub(crate) use session::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
pub(crate) use turn_kind::InterrogationTurnKind;
