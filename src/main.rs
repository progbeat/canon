#![cfg_attr(test, allow(dead_code, unused_imports))]

mod app;
mod check;
mod cli;
mod config_types;
mod evaluator;
mod fs_util;
mod gate;
mod git;
mod hash;
mod hooks;
mod isolation;
mod json_util;
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
mod xpec_state;

fn main() {
    cli::main();
}
