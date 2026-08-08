#![cfg_attr(test, allow(dead_code, unused_imports))]

mod app;
mod check;
mod cli;
mod config_types;
mod evaluator;
mod evaluator_sandbox_filesystem;
mod fs_util;
mod gate;
mod git;
mod hash;
mod hooks;
mod init;
mod isolation;
mod json_util;
mod logs;
mod materialization;
mod memoize;
mod notes;
mod output;
mod path_io_error;
mod platform;
mod process_cwd;
mod project;
mod project_types;
mod repo_inspection;
mod scope;
mod state_paths;
mod time;
mod token_usage;
mod xpec_state;

fn main() {
    cli::main();
}
