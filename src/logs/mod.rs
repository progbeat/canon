mod config;
mod error;
mod events;
mod fs;
mod lock;
mod render;
mod rotation;
mod writer;

pub(crate) use config::{thread_reuse_config, ThreadReuseConfig};
pub(crate) use error::{external_log_error, DiagnosticLogError, DiagnosticLogResult};
pub(crate) use events::{
    write_agent_failure_event, write_agent_missing_usage_event, write_agent_request_event,
    write_agent_response_event, write_cache_cleanup_event, write_check_finish_event,
    write_check_start_event, write_query_finish_event, write_query_start_event,
    write_thread_lifecycle_event, write_thread_restart_event, AgentTurnLogRequest,
    ThreadLifecycleEventFields, ThreadRestartEventFields,
};
pub(crate) use render::push_json_control_escape;
pub(crate) use writer::{DiagnosticLogWriter, DiagnosticRecordEvent};
