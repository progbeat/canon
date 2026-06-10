// Check execution is one workflow component. The leaf modules separate the
// run queue, per-expectation execution, and report accounting phases behind
// this facade.
mod expectation;
mod report;
mod run;

use crate::check::run::lazy_reset::LazyFullScopeResetCache;
use crate::git::VisibleTreeOidCache;
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use std::io::Write;

pub(crate) use run::run_check_with_runner_and_caches;

pub(crate) struct CheckRunCaches {
    pub(crate) history: HistoryCache,
    pub(crate) lazy_reset: LazyFullScopeResetCache,
    pub(crate) visible_tree_oid: VisibleTreeOidCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            history: HistoryCache::default(),
            lazy_reset: LazyFullScopeResetCache::default(),
            visible_tree_oid: VisibleTreeOidCache::new(),
        }
    }
}

pub(crate) struct CheckRunSideEffects<'out, 'cache, 'log> {
    pub(crate) diagnostic_log: Option<&'log mut DiagnosticLogWriter>,
    pub(crate) result_output: Option<&'out mut dyn Write>,
    pub(crate) live_progress_output: Option<crate::check::command::output::SharedCheckOutput>,
    pub(crate) caches: &'cache mut CheckRunCaches,
}
