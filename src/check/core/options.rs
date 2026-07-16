use super::expectation::ResolvedExpectation;
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) struct CheckOptions {
    // CLI-expanded candidates before cache filtering determines the final
    // evaluator queue.
    pub(crate) candidate_expectations: Vec<ResolvedExpectation>,
    pub(crate) selectors_provided: bool,
    // `--keep-going` continues evaluator work after failed results; it does
    // not bypass default cache-based selection.
    pub(crate) keep_going: bool,
    pub(crate) break_after_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawCheckOptions {
    pub(crate) keep_going: bool,
    pub(crate) break_after_tokens: Option<u64>,
    pub(crate) selectors: Vec<OsString>,
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    pub(crate) against_tree_explicit: bool,
    pub(crate) in_place: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) options: RawCheckOptions,
}

pub(crate) struct AskCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    pub(crate) against_tree_explicit: bool,
    pub(crate) in_place: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) question: String,
    pub(crate) default_agent_preset: Option<String>,
    pub(crate) query_scope: Vec<String>,
    pub(crate) query_scope_provided: bool,
}
