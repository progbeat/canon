mod expectation;
mod progress;
mod report;
mod run;

#[cfg(test)]
use crate::check::core::types::{CheckOptions, CheckRunError, CheckRunReport};
#[cfg(test)]
use crate::check::interrogation::state::CheckRuntime;
#[cfg(test)]
use crate::config_types::CheckConfig;
#[cfg(test)]
use crate::evaluator::EvaluatorRunner;
use crate::git::VisibleTreeOidCache;
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
#[cfg(test)]
use std::collections::BTreeSet;
use std::io::Write;
#[cfg(test)]
use std::path::Path;
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

#[cfg(test)]
pub(crate) fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: Option<&mut dyn Write>,
) -> Result<CheckRunReport, CheckRunError> {
    let mut caches = CheckRunCaches::new();
    let runtime = CheckRuntime::fixed(root, snapshot_root, config);
    let active_lazy_full_scope_reset_ids = BTreeSet::new();
    run_check_with_runner_and_caches(
        runtime,
        options,
        &active_lazy_full_scope_reset_ids,
        runner,
        CheckRunSideEffects {
            diagnostic_log,
            result_output,
            progress_output: None,
            started: Instant::now(),
            caches: &mut caches,
        },
    )
}
