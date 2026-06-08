use super::expectation::SelectedExpectation;
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) struct CheckOptions {
    // CLI-expanded selected expectations before check-only work-saving filters.
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) selectors_provided: bool,
    // `--keep-going` continues after non-pass results among selected
    // expectations; it does not bypass default cache-based selection.
    pub(crate) keep_going: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawCheckOptions {
    pub(crate) keep_going: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
    pub(crate) selectors: Vec<OsString>,
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    pub(crate) against_tree_explicit: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) query: Option<String>,
    pub(crate) query_preset: Option<String>,
    pub(crate) query_scope: Vec<String>,
    pub(crate) options: RawCheckOptions,
}
