#![cfg_attr(test, allow(dead_code, unused_imports))]

mod app;
mod check;
mod cli;
mod config_types;
mod evaluator;
mod evidence;
mod fs_util;
mod gate;
mod git;
mod hash;
mod history;
mod isolation;
mod json_util;
// Cache answer history is implemented end-to-end in `history::store`: path/read
// cache, answer-only durable JSONL writes, required field order, and
// probabilistic retention. `history::reuse` owns same-tree/cooldown lookup.
mod hooks;
mod logs;
mod notes;
mod output;
mod path_io_error;
mod platform;
mod project;
mod project_types;
mod repo_inspection;
mod scope;
mod staged;
mod state_paths;
mod time;
mod token_usage_types;

fn main() {
    cli::main();
}
