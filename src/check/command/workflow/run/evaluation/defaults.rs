use super::super::super::failure::{fail_check_before_selection, or_finalize, CheckFailureOutput};
use super::super::lifecycle::CheckCommandInspection;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::prepare::resolve_git_backed_tree_state;
use crate::check::config::collect_check_config;
use crate::check::CHECK_PATH;
use crate::cli::CommandError;
use crate::git::{TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

fn collect_default_pending_failure_output(
    root: &Path,
    output: CheckFailureOutput,
    inspection: &mut CheckCommandInspection,
    checked_tree: &TreeSource,
) -> CheckFailureOutput {
    let collection = collect_check_config(
        &mut inspection.repo_inspection,
        root,
        Path::new(CHECK_PATH),
        checked_tree,
    );
    let mut output = output;
    match collection {
        // [kK] Successful expansion establishes the collected set even when
        // subsequent config validation rejects that set.
        Ok(config) => output.mark_collection_complete(config.expectation_count()),
        // [kK] A collection failure is actionable configuration feedback, not
        // an evaluation that a later invocation can continue unchanged.
        Err(_) => output.mark_collection_failed(),
    }
    output
}

pub(in super::super) fn prepare_default_failure_output(
    root: &Path,
    output: CheckFailureOutput,
    in_place: bool,
    inspection: &mut CheckCommandInspection,
) -> CheckFailureOutput {
    // [90] In-place failure reporting has no Git-backed fallback: this boundary
    // returns before the default staged config, tree OIDs, or HEAD are read.
    if in_place {
        return output;
    }
    if !output.needs_pending_collection() {
        return output;
    }
    let tree_state = resolve_git_backed_tree_state(
        root,
        STAGED_TREE_ARG,
        DEFAULT_AGAINST_TREE_ARG,
        &mut inspection.repo_inspection,
        &inspection.git_resources,
    )
    .ok();
    // [kK,KD] The default snapshot is best-effort after an earlier preparation
    // failure. If it is unavailable, retain the original diagnostic and
    // required trailer instead of turning fallback reporting into a panic.
    match tree_state.as_ref() {
        Some(tree_state) => collect_default_pending_failure_output(
            root,
            output,
            inspection,
            &tree_state.checked_tree,
        ),
        None => output,
    }
}

pub(super) fn fail_check_with_default_output(
    root: &Path,
    in_place: bool,
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
    inspection: &mut CheckCommandInspection,
    error: String,
) -> Result<(), CommandError> {
    *failure_output = prepare_default_failure_output(root, *failure_output, in_place, inspection);
    fail_check_before_selection(
        diagnostic_log,
        public_output_progress,
        failure_output,
        error,
    )
}

pub(in super::super) fn or_fail_with_default_output<T, E: ToString>(
    result: Result<T, E>,
    root: &Path,
    in_place: bool,
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
    inspection: &mut CheckCommandInspection,
) -> Result<T, CommandError> {
    or_finalize(result, |err| {
        fail_check_with_default_output(
            root,
            in_place,
            diagnostic_log,
            public_output_progress,
            failure_output,
            inspection,
            err.to_string(),
        )
    })
}
