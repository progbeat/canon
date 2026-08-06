//! Runtime-log adapters at the interrogation boundary.

mod agent;
mod lifecycle;

pub(crate) use agent::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
pub(crate) use lifecycle::{
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
