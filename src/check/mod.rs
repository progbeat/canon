//! The `check` component and its deliberately partitioned implementation.
//!
//! Its subdirectories are cohesive lifecycle areas, not flat sibling files:
//! `command` owns CLI execution and output, `config` resolves inputs, `core`
//! owns shared domain values, `interrogation` talks to evaluators, and `engine`
//! evaluates selected expectations. `expectation_inspection` owns the shared
//! behavior behind the `show` command and evaluator tool. This root is their
//! component boundary and exposes only the small surface needed elsewhere.

pub(crate) const CHECK_PATH: &str = ".canon/check.yml";

mod cli_args;
mod command;
mod config;
mod core;
mod engine;
mod expectation_inspection;
mod interrogation;
mod q_scope;
mod run_caches;
mod show;

pub(crate) use command::{
    ask_help_command, check_help_command, run_ask_command, run_check_command,
};
pub(crate) use config::{
    is_missing_default_config_error, load_ask_config, load_check_config, load_in_place_ask_config,
};
pub(crate) use core::{
    evaluator_response_output_schema_for_scope, for_each_unique_report_record,
    parse_evaluator_response_for_short_id, CheckRecord, CheckRecordOutcome, CheckResult,
    CheckRunReport, EvaluatorResponseParseError, EvaluatorResponseSchemaScope, ParsedAnswer,
    ResolvedExpectation, INTERNAL_ERROR_UNPARSABLE,
};
#[cfg(test)]
pub(crate) use core::{EvaluationAnswer, ResolvedExpectationKind};
pub(crate) use engine::{
    expectation_identities, minimal_unique_expectation_prefix,
    order_selected_by_rank_and_latest_fail, run_check_with_runner_and_caches,
    run_temporary_expectation_interrogation, select_expectations_with_identities,
    CheckRunSideEffects, ExpectationIdentity, ResolveSelectedDiffFromTreeOids,
    TemporaryExpectationInterrogationContext,
};
pub(crate) use interrogation::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
pub(crate) use run_caches::CheckRunCaches;
pub(crate) use show::{run_show_command, show_help_command};
