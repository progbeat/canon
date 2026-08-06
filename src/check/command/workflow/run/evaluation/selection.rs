use crate::app::LazyAppServerRunner;
use crate::check::command::output::CheckFeedbackContext;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    or_fail_at_selection_boundary, start_check_with_candidates_or_fail, CheckFailureOutput,
    SelectionBoundary,
};
use crate::check::core::{CheckOptions, CheckRunReport, RawCheckOptions};
use crate::check::engine::selection::{
    expectation_identities, resolve_check_options_with_identities,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::{CheckRunCaches, ExpectationIdentity, ResolveSelectedDiffFromTreeOids};
use crate::cli::CommandError;
use crate::logs::{write_xpec_state_retention_event, DiagnosticLogWriter};
use crate::xpec_state::XpecStateCache;
use std::path::Path;
use std::time::Instant;

pub(super) struct CheckSelection {
    pub(super) identities: Vec<ExpectationIdentity>,
    pub(super) options: CheckOptions,
}

pub(super) struct PreparedCheckRun<'runtime, 'resources> {
    pub(super) runtime: CheckRuntime<'runtime>,
    pub(super) options: &'resources CheckOptions,
    pub(super) runner: &'resources mut LazyAppServerRunner,
    pub(super) check_caches: &'resources mut CheckRunCaches,
    pub(super) diagnostic_log: &'resources mut DiagnosticLogWriter,
    pub(super) started: Instant,
    pub(super) public_output_progress: &'resources mut CheckPublicOutputProgress,
    pub(super) progress_report: &'resources mut CheckRunReport,
    pub(super) feedback_context: Option<CheckFeedbackContext>,
    pub(super) resolve_selected_diff_from_tree_oids:
        Option<&'resources mut ResolveSelectedDiffFromTreeOids<'resources>>,
}

pub(super) fn resolve_check_selection(
    config: &crate::config_types::CheckConfig,
    raw_options: &RawCheckOptions,
) -> Result<CheckSelection, String> {
    let identities = expectation_identities(config)?;
    let options = resolve_check_options_with_identities(config, &identities, raw_options)?;
    Ok(CheckSelection {
        identities,
        options,
    })
}

pub(super) fn record_collected_expectations(
    expectation_count: usize,
    failure_output: &mut CheckFailureOutput,
    progress_report: &mut CheckRunReport,
) {
    // [w,2Z] Failure state and the live report describe the same collected
    // outcome domain. Update them together so every check mode presents the
    // same pending count before evaluation starts.
    failure_output.mark_collection_complete(expectation_count);
    *progress_report = CheckRunReport {
        records: Vec::new(),
        cached_passes: Vec::new(),
        pending: expectation_count,
    };
}

pub(super) fn start_check_with_candidates(
    selection: &CheckSelection,
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    let candidate_ids = selection
        .options
        .candidate_expectations
        .iter()
        .map(|expectation| expectation.require_configured_id().map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    start_check_with_candidates_or_fail(
        diagnostic_log,
        candidate_ids,
        public_output_progress,
        failure_output,
    )
}

pub(super) fn retain_only_current_configuration_xpec_state(
    xpec_state: &mut XpecStateCache,
    root: &Path,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    // [fh] Both normal stateful check modes call this after resolving every
    // current configuration identity and before any evaluation can write
    // per-ID state. Thus configuration changes cannot accumulate ID dirs.
    let (removed, kept) = or_fail_at_selection_boundary(
        xpec_state.retain_only_current_configuration(root, identities),
        SelectionBoundary::After,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    or_fail_at_selection_boundary(
        write_xpec_state_retention_event(diagnostic_log, removed, kept),
        SelectionBoundary::After,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    Ok(())
}
