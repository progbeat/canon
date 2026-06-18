pub(crate) const CHECK_PATH: &str = ".canon/check.yml";

// The check component is split by lifecycle phase: command handling, config
// loading, shared records, evaluator interrogation, run orchestration, and show
// rendering. This root exposes the small surface other components need.
mod command;
mod config;
mod core;
mod interrogation;
mod run;
mod show;

pub(crate) use command::preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
pub(crate) use command::{check_help_command, run_check_command};
pub(crate) use config::{
    codex_reasoning_effort, expand_staged_generator_paths_from_listing,
    parse_tree_check_config_content_with_root,
};
pub(crate) use core::{
    evaluator_response_output_schema, matches_answer_pattern, parse_evaluator_response,
    CheckRecord, CheckRecordOutcome, CheckResult, ParsedAnswer, SelectedExpectation,
    INTERNAL_ERROR_UNPARSABLE,
};
pub(crate) use interrogation::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
pub(crate) use run::{
    expectation_identities, run_check_with_runner_and_caches, select_expectations_with_identities,
    CheckRunCaches, CheckRunSideEffects, ExpectationIdentity,
};
pub(crate) use show::{run_show_command, show_help_command};
