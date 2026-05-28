const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
// Canon-owned persistent state is rooted at
// `CANON_STATE_DIR = git rev-parse --git-path canon`.
const CANON_STATE_DIR_GIT_PATH: &str = "canon";
// `${CANON_STATE_DIR}/cache`, resolved through `git rev-parse --git-path`.
const CANON_CACHE_DIR_GIT_PATH: &str = "canon/cache";
// `${CANON_STATE_DIR}/logs`, resolved through `git rev-parse --git-path`.
const CANON_LOG_DIR_GIT_PATH: &str = "canon/logs";
const DEFAULT_DIAGNOSTIC_LOG_FILES: [&str; 8] = [
    "0.jsonl", "1.jsonl", "2.jsonl", "3.jsonl", "4.jsonl", "5.jsonl", "6.jsonl", "7.jsonl",
];
const DEFAULT_DIAGNOSTIC_LOG_CONFIG: DiagnosticLogConfig = DiagnosticLogConfig {
    max_bytes: 0,
    explicitly_disabled: true,
    files: &DEFAULT_DIAGNOSTIC_LOG_FILES,
};
const HISTORY_COMPACT_KEEP_RECORDS: usize = 8;
const HISTORY_COMPACT_CHANCE_DENOMINATOR: u64 = 16;
const APP_SERVER_TURN_TIMEOUT_SECS: u64 = 300;
const DEFAULT_CHECK_TEMPLATE: &str = include_str!("../.canon/templates/default/check.yml");
// `${CANON_STATE_DIR}/hooks`, resolved through `git rev-parse --git-path`.
const GIT_HOOKS_PATH: &str = "canon/hooks";
// `${CANON_STATE_DIR}/hooks/pre-commit`, resolved through `git rev-parse --git-path`.
const PRE_COMMIT_HOOK_PATH: &str = "canon/hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../resources/git-hooks/pre-commit");
const RESULT_PASS: &str = "pass";
const RESULT_FAIL: &str = "fail";
const ERROR_INSUFFICIENT_EVIDENCE: &str = "insufficient-evidence";
const ERROR_INVALID_QUESTION: &str = "invalid-question";
const ERROR_UNPARSABLE: &str = "unparsable";

pub(crate) struct DiagnosticLogConfig {
    pub(crate) max_bytes: u64,
    pub(crate) explicitly_disabled: bool,
    pub(crate) files: &'static [&'static str],
}

#[path = "app/app_server.rs"]
mod app_server;
#[path = "app/app_server_io.rs"]
mod app_server_io;
#[path = "app/app_server_process.rs"]
mod app_server_process;
#[path = "app/app_server_protocol.rs"]
mod app_server_protocol;
#[path = "app/app_server_runner.rs"]
mod app_server_runner;
#[path = "app/app_server_transport.rs"]
mod app_server_transport;
#[path = "app/app_server_usage.rs"]
mod app_server_usage;
#[path = "checking/check.rs"]
mod check;
#[path = "checking/check_cache.rs"]
mod check_cache;
#[path = "checking/check_command.rs"]
mod check_command;
#[path = "checking/check_command_args.rs"]
mod check_command_args;
#[path = "checking/check_command_finish.rs"]
mod check_command_finish;
#[path = "checking/check_config.rs"]
mod check_config;
// Implementation files live under domain subdirectories. The crate root keeps
// stable module names with `#[path]` so existing internal imports do not churn
// when files move between navigation groups.
#[path = "checking/check_config_expansion.rs"]
mod check_config_expansion;
#[path = "checking/check_errors.rs"]
mod check_errors;
#[path = "checking/check_generator_paths.rs"]
mod check_generator_paths;
#[path = "checking/check_interrogation.rs"]
mod check_interrogation;
#[path = "checking/check_interrogation_policy.rs"]
mod check_interrogation_policy;
#[path = "checking/check_interrogation_records.rs"]
mod check_interrogation_records;
#[path = "checking/check_interrogation_state.rs"]
mod check_interrogation_state;
#[path = "checking/check_lazy_reset.rs"]
mod check_lazy_reset;
#[path = "checking/check_model_fallback.rs"]
mod check_model_fallback;
#[path = "checking/check_narrowing.rs"]
mod check_narrowing;
#[path = "checking/check_order_state.rs"]
mod check_order_state;
#[path = "checking/check_output.rs"]
mod check_output;
#[path = "checking/check_preflight.rs"]
mod check_preflight;
#[path = "checking/check_query.rs"]
mod check_query;
#[path = "checking/check_query_command.rs"]
mod check_query_command;
#[path = "checking/check_reporting.rs"]
mod check_reporting;
#[path = "checking/check_selection.rs"]
mod check_selection;
#[path = "checking/check_types.rs"]
mod check_types;
#[path = "checking/check_validation.rs"]
mod check_validation;
mod cli;
mod config_types;
#[path = "evaluator_runtime/evaluator.rs"]
mod evaluator;
#[path = "evaluator_runtime/evaluator_config.rs"]
mod evaluator_config;
#[path = "evaluator_runtime/evaluator_prompt.rs"]
mod evaluator_prompt;
#[path = "evaluator_runtime/evaluator_response.rs"]
mod evaluator_response;
#[path = "evaluator_runtime/evaluator_response_cache.rs"]
mod evaluator_response_cache;
#[path = "evaluator_runtime/evaluator_scope.rs"]
mod evaluator_scope;
#[path = "evaluator_runtime/evaluator_turn.rs"]
mod evaluator_turn;
#[path = "evaluator_runtime/evaluator_types.rs"]
mod evaluator_types;
mod fs_util;
mod gate;
#[path = "git_runtime/git.rs"]
mod git;
#[path = "git_runtime/git_config.rs"]
mod git_config;
mod hash;
#[path = "history_store/history.rs"]
mod history;
#[path = "history_store/history_append.rs"]
mod history_append;
#[path = "history_store/history_cache_key.rs"]
mod history_cache_key;
#[path = "history_store/history_cleanup.rs"]
mod history_cleanup;
#[path = "history_store/history_compaction.rs"]
mod history_compaction;
#[path = "history_store/history_reuse.rs"]
mod history_reuse;
mod hooks;
#[path = "runtime_logs/logging.rs"]
mod logging;
#[path = "runtime_logs/logging_config.rs"]
mod logging_config;
#[path = "runtime_logs/logging_error.rs"]
mod logging_error;
#[path = "runtime_logs/logging_fs.rs"]
mod logging_fs;
#[path = "runtime_logs/logging_lock.rs"]
mod logging_lock;
#[path = "runtime_logs/logging_render.rs"]
mod logging_render;
#[path = "runtime_logs/logging_rotation.rs"]
mod logging_rotation;
#[path = "notes_store/notes.rs"]
mod notes;
#[path = "notes_store/notes_cli.rs"]
mod notes_cli;
#[path = "notes_store/notes_header.rs"]
mod notes_header;
#[path = "notes_store/notes_index.rs"]
mod notes_index;
#[path = "notes_store/notes_restore.rs"]
mod notes_restore;
mod output;
mod path_io_error;
mod platform;
mod project;
mod project_types;
mod repo_inspection;
mod scope;
#[path = "staged_snapshot/staged_worktree.rs"]
mod staged_worktree;
#[path = "staged_snapshot/staged_worktree_paths.rs"]
mod staged_worktree_paths;
mod time;
mod token_usage_types;
#[path = "git_runtime/visible_tree_oid.rs"]
mod visible_tree_oid;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;
