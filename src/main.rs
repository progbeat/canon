#![cfg_attr(test, allow(dead_code, unused_imports))]

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
// The `canon init` seed is compiled into the binary from a check-config source
// file, not loaded at runtime as an evaluator interrogation prompt/instruction.
// Interrogation texts live under `resources/prompts/`.
const DEFAULT_CHECK_CONFIG_SOURCE: &str = include_str!("../.canon/templates/default/check.yml");
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

mod app {
    #[path = "io.rs"]
    pub(crate) mod io;
    #[path = "process.rs"]
    pub(crate) mod process;
    #[path = "protocol.rs"]
    pub(crate) mod protocol;
    #[path = "runner.rs"]
    pub(crate) mod runner;
    #[path = "server.rs"]
    pub(crate) mod server;
    #[path = "transport.rs"]
    pub(crate) mod transport;
    #[path = "usage.rs"]
    pub(crate) mod usage;
}

// Glossary implementation map for `canon check`:
// - expectation collection/identity: `check::config_expansion`, `check::selection`, `hash`.
// - scope and scoped tree semantics: `scope`, `git::visible_tree_oid`, `staged::worktree`.
// - q-scope storage/reuse: `history::store` writes `qScope`, `history::reuse` seeds the next
//   visible scope from the latest answer-history q-scope.
// - q-scope suggestion lifecycle: `evaluator::response` parses the evaluator claim,
//   `check::interrogation_policy` verifies whether it becomes a reusable q-scope,
//   and `check::run` records only verified answer scopes in history.
// - visible scope/tree formation: `check::interrogation_state` chooses the stored
//   q-scope or full scope, `scope::effective_ignore_patterns` applies configured
//   ignore rules, and `staged::worktree` materializes the resulting visible tree.
// - evidence and answer/error records: `evaluator::response`,
//   `check::interrogation_records`, `check::types`, and `check::output`.
// - evaluator thread reuse: `check::interrogation_state::evaluator_thread_reuse_key`
//   and `check::interrogation` keep reusable threads scoped by model, visible tree,
//   and developer-instruction inputs.
mod check {
    #[path = "run.rs"]
    pub(crate) mod run;
    pub(crate) use run::*;
    #[path = "cache.rs"]
    pub(crate) mod cache;
    #[path = "command.rs"]
    pub(crate) mod command;
    #[path = "command_args.rs"]
    pub(crate) mod command_args;
    #[path = "command_finish.rs"]
    pub(crate) mod command_finish;
    #[path = "config.rs"]
    pub(crate) mod config;
    #[path = "config_expansion.rs"]
    pub(crate) mod config_expansion;
    #[path = "errors.rs"]
    pub(crate) mod errors;
    #[path = "generator_paths.rs"]
    pub(crate) mod generator_paths;
    #[path = "interrogation.rs"]
    pub(crate) mod interrogation;
    #[path = "interrogation_policy.rs"]
    pub(crate) mod interrogation_policy;
    #[path = "interrogation_records.rs"]
    pub(crate) mod interrogation_records;
    #[path = "interrogation_state.rs"]
    pub(crate) mod interrogation_state;
    #[path = "lazy_reset.rs"]
    pub(crate) mod lazy_reset;
    #[path = "model_fallback.rs"]
    pub(crate) mod model_fallback;
    #[path = "narrowing.rs"]
    pub(crate) mod narrowing;
    #[path = "order_state.rs"]
    pub(crate) mod order_state;
    #[path = "output.rs"]
    pub(crate) mod output;
    #[path = "preflight.rs"]
    pub(crate) mod preflight;
    #[path = "query.rs"]
    pub(crate) mod query;
    #[path = "query_command.rs"]
    pub(crate) mod query_command;
    #[path = "reporting.rs"]
    pub(crate) mod reporting;
    #[path = "selection.rs"]
    pub(crate) mod selection;
    #[path = "types.rs"]
    pub(crate) mod types;
    #[path = "validation.rs"]
    pub(crate) mod validation;
}
mod cli;
mod config_types;
mod evaluator {
    #[path = "core.rs"]
    pub(crate) mod core;
    pub(crate) use core::*;
    #[path = "config.rs"]
    pub(crate) mod config;
    #[path = "prompt.rs"]
    pub(crate) mod prompt;
    #[path = "response.rs"]
    pub(crate) mod response;
    #[path = "response_cache.rs"]
    pub(crate) mod response_cache;
    #[path = "scope.rs"]
    pub(crate) mod scope;
    #[path = "turn.rs"]
    pub(crate) mod turn;
    #[path = "types.rs"]
    pub(crate) mod types;
}
mod fs_util;
mod gate;
mod git {
    #[path = "program.rs"]
    pub(crate) mod program;
    pub(crate) use program::*;
    #[path = "config.rs"]
    pub(crate) mod config;
    #[path = "tree_source.rs"]
    pub(crate) mod tree_source;
    #[path = "visible_tree_oid.rs"]
    pub(crate) mod visible_tree_oid;
}
mod hash;
mod history {
    #[path = "store.rs"]
    pub(crate) mod store;
    pub(crate) use store::*;
    #[path = "cache_key.rs"]
    pub(crate) mod cache_key;
    #[path = "cleanup.rs"]
    pub(crate) mod cleanup;
    #[path = "reuse.rs"]
    pub(crate) mod reuse;
}
mod isolation;
// Cache answer history is implemented end-to-end in `history::store`: path/read
// cache, answer-only durable JSONL writes, required field order, and
// probabilistic retention. `history::reuse` owns same-tree/cooldown lookup.
mod hooks;
mod logs {
    #[path = "writer.rs"]
    pub(crate) mod writer;
    pub(crate) use writer::*;
    #[path = "config.rs"]
    pub(crate) mod config;
    #[path = "error.rs"]
    pub(crate) mod error;
    #[path = "fs.rs"]
    pub(crate) mod fs;
    #[path = "lock.rs"]
    pub(crate) mod lock;
    #[path = "render.rs"]
    pub(crate) mod render;
    pub(crate) use render::push_json_control_escape;
    #[path = "rotation.rs"]
    pub(crate) mod rotation;
}
mod notes {
    #[path = "lock.rs"]
    pub(crate) mod lock;
    #[path = "store.rs"]
    pub(crate) mod store;
    pub(crate) use store::*;
    #[path = "cli.rs"]
    pub(crate) mod cli;
    #[path = "header.rs"]
    pub(crate) mod header;
    #[path = "index.rs"]
    pub(crate) mod index;
    #[path = "restore.rs"]
    pub(crate) mod restore;
}
mod output;
mod path_io_error;
mod platform;
mod project;
mod project_types;
mod repo_inspection;
mod scope;
mod staged {
    #[path = "worktree.rs"]
    pub(crate) mod worktree;
    pub(crate) use worktree::*;
    #[path = "paths.rs"]
    pub(crate) mod paths;
}
mod time;
mod token_usage_types;

fn main() {
    cli::main();
}
