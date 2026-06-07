mod expectation;
mod progress;
mod report;
mod run;

use crate::git::VisibleTreeOidCache;
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use std::io::Write;
use std::time::Instant;

pub(crate) use run::run_check_with_runner_and_caches;

pub(crate) struct CheckRunCaches {
    pub(crate) history: HistoryCache,
    pub(crate) visible_tree_oid: VisibleTreeOidCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            history: HistoryCache::default(),
            visible_tree_oid: VisibleTreeOidCache::new(),
        }
    }
}

pub(crate) struct CheckRunSideEffects<'out, 'cache, 'log> {
    pub(crate) diagnostic_log: Option<&'log mut DiagnosticLogWriter>,
    pub(crate) result_output: Option<&'out mut dyn Write>,
    pub(crate) progress_output: Option<crate::check::command::output::SharedCheckOutput>,
    pub(crate) started: Instant,
    pub(crate) caches: &'cache mut CheckRunCaches,
}
