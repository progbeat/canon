//! One evaluator-turn component.
//!
//! Configuration selects the turn, protocol renders and parses it, progress
//! reports it, and `turn` coordinates those stages behind this module's API.

mod config;
mod inspection;
mod progress;
mod project_tool;
mod protocol;
mod tool_schema;
mod turn;

pub(crate) use config::{
    app_server_args, evaluator_reasoning_effort, evaluator_thread_config_identity, AppServerArgs,
    EphemeralEvaluatorThreadPermissionProfile, EvaluatorHostIsolation, EvaluatorProcessIsolation,
    EvaluatorRuntimeConfigContext, EvaluatorRuntimeConfigSnapshot, EvaluatorThreadConfigIdentity,
    EvaluatorThreadConfigIdentityContext,
};
pub(crate) use inspection::{
    EvaluatorInspectionDynamicToolHandler, EvaluatorProjectFilesystem,
    ReadOnlyProjectInspectionPlan,
};
pub(crate) use progress::{
    EvaluatorProgress, EvaluatorProgressMarker, PROGRESS_TIMELINE_MARKER_INTERVAL,
};
pub(crate) use protocol::{
    canon_show_dynamic_tools, developer_instructions_cache_key, evaluator_base_instructions,
    BaseInstructionsContext, DeveloperInstructionsCacheKey, DeveloperInstructionsContext,
    EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult,
    EvaluatorError, EvaluatorPromptMode, EvaluatorRunner, EvaluatorTurnPromptContext,
    InvocationResponseParseMemo, PromptRenderer, RenderedPrompt,
};
pub(crate) use turn::{
    ask_once, evaluator_models, is_interrupted, is_technical_failure, record_from_response,
    write_thread_lifecycle_event, write_thread_restart_event, EvaluatorAttempt,
    EvaluatorAttemptReason, EvaluatorAttemptRequest, EvaluatorAttemptSequence,
    EvaluatorFailureKind, EvaluatorTurnContext, ParsedTurnResponse, ThreadEvaluationLogContext,
    ThreadLifecycleLog,
};
