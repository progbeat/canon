use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::atomic::Ordering;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
const GIT_CANON_CACHE_DIR: &str = "canon/cache";
const GIT_CANON_LOG_DIR: &str = "canon/logs";
const DEFAULT_DIAGNOSTIC_LOG_FILES: [&str; 8] = [
    "0.jsonl", "1.jsonl", "2.jsonl", "3.jsonl", "4.jsonl", "5.jsonl", "6.jsonl", "7.jsonl",
];
const DEFAULT_DIAGNOSTIC_LOG_CONFIG: DiagnosticLogConfig = DiagnosticLogConfig {
    max_bytes: 0,
    files: &DEFAULT_DIAGNOSTIC_LOG_FILES,
};
const HISTORY_COMPACT_KEEP_RECORDS: usize = 5;
const HISTORY_COMPACT_CHANCE_DENOMINATOR: u64 = 15;
const APP_SERVER_TURN_TIMEOUT_SECS: u64 = 300;
const DEFAULT_CHECK_TEMPLATE: &str = include_str!("../.canon/templates/default/check.yml");
const GIT_HOOKS_PATH: &str = ".git/canon/hooks";
const PRE_COMMIT_HOOK_PATH: &str = ".git/canon/hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../resources/git-hooks/pre-commit");
const RESULT_PASS: &str = "pass";
const RESULT_FAIL: &str = "fail";
const OBSERVED_IDK: &str = "idk";
const OBSERVED_MALFORMED: &str = "malformed";
const UNPARSEABLE_OBSERVED: &str = "unparseable";
const EMPTY_EVIDENCE_OBSERVED: &str = "empty-evidence";
static CHECK_INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub(crate) struct DiagnosticLogConfig {
    pub(crate) max_bytes: u64,
    pub(crate) files: &'static [&'static str],
}

#[cfg(unix)]
// Unix signal and process-group APIs are not exposed by Rust's standard
// library. The safe wrappers in `check_preflight` and `app_server_process`
// keep all FFI calls behind return-value checks and narrow safety comments.
unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(unix)]
extern "C" fn handle_sigint(_: i32) {
    // This handler only stores to an atomic flag; it does not allocate, lock,
    // or touch non-async-signal-safe state.
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

mod app_server;
mod app_server_process;
mod app_server_protocol;
mod app_server_runner;
mod app_server_transport;
mod check;
mod check_cache;
mod check_command;
mod check_command_args;
mod check_command_finish;
mod check_config;
// Module declarations are concrete ownership boundaries: shared check/config,
// evaluator, project, and token-usage types live in their subsystem modules,
// while generator template rendering belongs to check_config_expansion.
mod check_config_expansion;
mod check_errors;
mod check_generator_paths;
mod check_interrogation;
mod check_interrogation_policy;
mod check_interrogation_records;
mod check_interrogation_state;
mod check_lazy_reset;
mod check_model_fallback;
mod check_narrowing;
mod check_order_state;
mod check_output;
mod check_preflight;
mod check_query;
mod check_query_command;
mod check_reporting;
mod check_selection;
mod check_types;
mod check_validation;
mod cli;
mod config_types;
mod evaluator;
mod evaluator_config;
mod evaluator_json;
mod evaluator_prompt;
mod evaluator_response;
mod evaluator_response_cache;
mod evaluator_scope;
mod evaluator_turn;
mod evaluator_types;
mod fs_util;
mod gate;
mod git;
mod git_config;
mod hash;
mod history;
mod history_append;
mod history_cache_key;
mod history_cleanup;
mod history_compaction;
mod history_reuse;
mod hooks;
mod logging;
mod logging_config;
mod logging_error;
mod logging_fs;
mod logging_lock;
mod logging_render;
mod logging_rotation;
mod notes;
mod notes_cli;
mod notes_header;
mod notes_index;
mod notes_restore;
mod output;
mod path_io_error;
mod project;
mod project_types;
mod repo_inspection;
mod scope;
mod scope_hash;
mod staged_worktree;
mod staged_worktree_git;
mod staged_worktree_paths;
mod staged_worktree_validate;
mod thread_reuse_config;
mod time;
mod token_usage_types;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;
