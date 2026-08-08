use super::super::CheckRunCaches;
use crate::check::command::output::SharedCheckOutput;
use crate::check::core::{
    assert_evaluation_postconditions, CheckOptions, CheckRecord, CheckRunReport,
    ResolvedExpectation,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::InterrogationSession;
use crate::evaluator::EvaluatorRunner;
use crate::logs::DiagnosticLogWriter;
use std::io::Write;

pub(in crate::check::engine::execute) struct CheckExpectationRunContext<
    'a,
    'out,
    'log,
    R: EvaluatorRunner,
> {
    pub(in crate::check::engine::execute) runtime: &'a CheckRuntime<'a>,
    pub(in crate::check::engine::execute) options: &'a CheckOptions,
    pub(in crate::check::engine::execute) runner: &'a mut R,
    pub(in crate::check::engine::execute) diagnostic_log:
        &'a mut Option<&'log mut DiagnosticLogWriter>,
    pub(in crate::check::engine::execute) result_output: &'a mut Option<&'out mut dyn Write>,
    pub(in crate::check::engine::execute) live_report_output: &'a Option<SharedCheckOutput>,
    pub(in crate::check::engine::execute) caches: &'a mut CheckRunCaches,
    pub(in crate::check::engine::execute) interrogation_session: &'a mut InterrogationSession,
    pub(in crate::check::engine::execute) progress_report: &'a mut CheckRunReport,
}

impl<R: EvaluatorRunner> CheckExpectationRunContext<'_, '_, '_, R> {
    pub(in crate::check::engine::execute::expectation) fn cached_visible_tree_oid(
        &mut self,
        expectation: &ResolvedExpectation,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        self.runtime.visible_tree_oid(
            &mut self.caches.visible_tree_oid_cache,
            &expectation.agent,
            scope,
        )
    }

    pub(in crate::check::engine::execute::expectation) fn record_completed(
        &mut self,
        record: &CheckRecord,
    ) {
        super::super::report::append_completed_record(
            self.progress_report,
            self.runtime.config.expectations.len(),
            record,
        );
    }
}

pub(in crate::check::engine::execute) struct CheckExpectationRunOutcome {
    pub(in crate::check::engine::execute) stop_run: bool,
    pub(in crate::check::engine::execute) interrupted: bool,
}

impl CheckExpectationRunOutcome {
    pub(super) fn after_evaluation(
        record: &CheckRecord,
        keep_going: bool,
        interrupted: bool,
    ) -> Self {
        CheckExpectationRunOutcome {
            stop_run: !keep_going && !record.passed(),
            interrupted,
        }
    }
}

pub(crate) struct TemporaryExpectationInterrogationContext<'a, 'log, R: EvaluatorRunner> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) runner: &'a mut R,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    pub(crate) caches: &'a mut CheckRunCaches,
    pub(crate) interrogation_session: &'a mut InterrogationSession,
}

pub(super) struct CompletedCheckInterrogation {
    pub(super) record: CheckRecord,
    pub(super) context_compaction_hit: bool,
    pub(super) interrupted: bool,
}

pub(super) fn assert_final_check_evaluation_postconditions(record: &CheckRecord) {
    assert_evaluation_postconditions(record.result, record.error.as_deref());
}
