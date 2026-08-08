// Check execution is one workflow component. The leaf modules separate the
// run queue, per-expectation execution, and report accounting phases behind
// this facade.
mod expectation;
mod persistence;
mod progress;
mod report;
mod run;
mod shell;
mod work_queue;

use crate::check::CheckRunCaches;
use crate::logs::DiagnosticLogWriter;
use crate::repo_inspection::RepoInspectionCache;
use std::collections::BTreeMap;
use std::io::Write;

pub(crate) use expectation::{
    run_temporary_expectation_interrogation, TemporaryExpectationInterrogationContext,
};
pub(crate) use run::run_check_with_runner_and_caches;

pub(crate) type ResolveSelectedDiffFromTreeOids<'a> = dyn FnMut(
        &[crate::check::core::ResolvedExpectation],
        &mut RepoInspectionCache,
    ) -> Result<BTreeMap<String, String>, String>
    + 'a;

pub(crate) struct CheckRunSideEffects<'out, 'cache, 'log, 'prepare> {
    pub(crate) diagnostic_log: Option<&'log mut DiagnosticLogWriter>,
    pub(crate) result_output: Option<&'out mut dyn Write>,
    pub(crate) live_report_output: Option<crate::check::command::output::SharedCheckOutput>,
    pub(crate) caches: &'cache mut CheckRunCaches,
    pub(crate) resolve_selected_diff_from_tree_oids:
        Option<&'prepare mut ResolveSelectedDiffFromTreeOids<'prepare>>,
}
