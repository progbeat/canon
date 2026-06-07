pub(crate) const CHECK_PATH: &str = ".canon/check.yml";

mod command;
mod config;
mod core;
mod interrogation;
mod run;

pub(crate) use command::preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
pub(crate) use command::{check_help_command, run_check_command};
pub(crate) use config::{
    codex_reasoning_effort, expand_staged_generator_paths_from_listing,
    parse_tree_check_config_content_with_root,
};
pub(crate) use core::{
    CheckRecord, CheckRecordOutcome, CheckResult, EvaluatorResponseJson, ParsedAnswer,
    SelectedExpectation, ERROR_INSUFFICIENT_EVIDENCE, ERROR_UNPARSABLE,
};
pub(crate) use interrogation::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
pub(crate) use run::{
    expectation_identities, run_check_with_runner_and_caches, select_expectations_with_identities,
    CheckRunCaches, CheckRunSideEffects, ExpectationIdentity,
};
