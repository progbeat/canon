pub(crate) const CHECK_PATH: &str = ".canon/check.yml";

mod cache;
mod command;
mod command_args;
mod command_finish;
mod config;
mod config_expansion;
mod errors;
mod generator_paths;
mod interrogation;
mod interrogation_policy;
mod interrogation_records;
mod interrogation_state;
mod lazy_reset;
mod model_fallback;
mod narrowing;
mod order_state;
mod output;
mod preflight;
mod query;
mod query_command;
mod reporting;
mod run;
mod selection;
mod types;
mod validation;

pub(crate) use command::run_check_command;
pub(crate) use command_args::{check_help_command, check_help_requested};
pub(crate) use config::parse_tree_check_config_content_with_root;
#[cfg(test)]
pub(crate) use generator_paths::expand_generator_paths;
pub(crate) use generator_paths::expand_staged_generator_paths_from_listing;
pub(crate) use preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
pub(crate) use run::{run_check_with_runner_and_caches, CheckRunCaches};
pub(crate) use selection::{
    expectation_identities, select_expectations_with_identities, ExpectationIdentity,
};
#[cfg(test)]
pub(crate) use types::Cooldown;
pub(crate) use types::{
    CheckRecord, CheckRecordOutcome, CheckResult, EvaluatorResponseJson, ParsedAnswer,
    SelectedExpectation, ERROR_UNPARSABLE,
};
pub(crate) use validation::codex_reasoning_effort;
