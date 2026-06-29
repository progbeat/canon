// Check execution is one workflow component. The leaf modules separate the
// run queue, per-expectation execution, and report accounting phases behind
// this facade.
mod expectation;
mod progress;
mod report;
mod run;

use crate::git::VisibleTreeOidCache;
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::collections::BTreeSet;
use std::io::Write;

pub(crate) use report::skipped_count;
pub(crate) use run::run_check_with_runner_and_caches;

pub(crate) struct CheckRunCaches {
    pub(crate) xpec_state: XpecStateCache,
    pub(crate) run_start_pass_ids: BTreeSet<String>,
    pub(crate) visible_tree_oid: VisibleTreeOidCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            xpec_state: XpecStateCache::default(),
            run_start_pass_ids: BTreeSet::new(),
            visible_tree_oid: VisibleTreeOidCache::new(),
        }
    }
}

pub(crate) struct CheckRunSideEffects<'out, 'cache, 'log> {
    pub(crate) diagnostic_log: Option<&'log mut DiagnosticLogWriter>,
    pub(crate) result_output: Option<&'out mut dyn Write>,
    pub(crate) live_report_output: Option<crate::check::command::output::SharedCheckOutput>,
    pub(crate) caches: &'cache mut CheckRunCaches,
}
