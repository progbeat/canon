use super::expectation::ResolvedExpectation;
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) struct CheckOptions {
    // CLI-expanded candidates are not yet Canon's Selected set. In default
    // Git-backed mode, cache classification removes reusable results before
    // the remaining candidates become the selected evaluator queue. In-place
    // mode has no Git state and therefore no Cached Result domain, so every
    // candidate is selected. Explicit selectors force every matching candidate
    // into the queue in either mode.
    pub(crate) candidate_expectations: Vec<ResolvedExpectation>,
    pub(crate) selectors_provided: bool,
    // `--keep-going` continues evaluator work after failed results; it does
    // not bypass default cache-based selection.
    pub(crate) keep_going: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawCheckOptions {
    pub(crate) keep_going: bool,
    pub(crate) selectors: Vec<OsString>,
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    // [kK] Feedback eligibility is defined by these resolved values, not by
    // whether the caller spelled an equal value explicitly.
    pub(crate) sources_have_command_default_values: bool,
    pub(crate) in_place: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) options: RawCheckOptions,
}

pub(crate) struct AskCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) config_explicit: bool,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    pub(crate) in_place: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) question: String,
    pub(crate) default_agent_preset: Option<String>,
}
