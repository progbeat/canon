use crate::check::command::workflow::failure::{
    fail_check_before_lifecycle, requested_check_output, CheckPublicOutputProgress,
};
mod diagnostics;
mod dispatch;
mod execution;
mod panic;
mod preflight;
mod terminal;

use super::root::{git_backed_diagnostic_log_plan, resolve_check_like_root};
use crate::check::command::args::parse_check_command_args;
use crate::check::command::workflow::trailer::check_command_emits_feedback;
use crate::check::command::{GitBackedCheckResources, TokenUsageSummary};
use crate::check::core::CheckRunReport;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogPlan;
use crate::repo_inspection::RepoInspectionCache;
use crate::state_paths::CanonStateRoot;
use diagnostics::{finish_check_command, start_check_diagnostic_log};
use execution::{run_check_command_with_writer, write_default_failure_output};
use panic::{resume_panicked_check, PanickedCheckContext};
use preflight::preparse_args_use_in_place;
use std::ffi::OsString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::time::Instant;

// Command execution coordinates CLI parsing, tree/config preparation, and
// final reporting. Per-expectation completion and last-result bookkeeping are
// delegated to the check-run execution layer.
pub(super) struct CheckCommandInspection {
    pub(super) repo_inspection: RepoInspectionCache,
    pub(super) git_resources: GitBackedCheckResources,
}

impl CheckCommandInspection {
    fn new() -> CheckCommandInspection {
        CheckCommandInspection {
            repo_inspection: RepoInspectionCache::new(),
            git_resources: GitBackedCheckResources::persistent(),
        }
    }
}

pub(super) fn run_check_command(args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    terminal::with_suppressed_terminal_echo(|| prepare_and_run_check_command(args, started))
}

fn prepare_and_run_check_command(args: &[OsString], started: Instant) -> Result<(), CommandError> {
    let command_root = match resolve_check_like_root(args) {
        Ok(command_root) => command_root,
        Err(err) => {
            return fail_check_before_lifecycle(
                requested_check_output(started, false),
                err.to_string(),
            );
        }
    };
    let diagnostic_log_plan = git_backed_diagnostic_log_plan(&command_root);
    // [1g,90] Resolve the command-wide output namespace independently of the
    // selected evaluation mode. A Git-derived default is only an opaque
    // control-plane path and never enters the in-place evaluator context.
    let command_persistent_state_root =
        match CanonStateRoot::resolve_if_available(&command_root.root) {
            Ok(command_persistent_state_root) => command_persistent_state_root,
            Err(err) => {
                let default_feedback_eligible =
                    parse_check_command_args(args, command_root.default_in_place)
                        .as_ref()
                        .is_ok_and(check_command_emits_feedback);
                return fail_check_before_lifecycle(
                    requested_check_output(started, default_feedback_eligible),
                    err,
                );
            }
        };
    run_prepared_check_command(
        &command_root.root,
        args,
        command_root.default_in_place,
        command_persistent_state_root,
        diagnostic_log_plan,
        started,
    )
}

fn run_prepared_check_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    command_persistent_state_root: Option<crate::state_paths::CanonStateRoot>,
    diagnostic_log_plan: Option<DiagnosticLogPlan>,
    started: Instant,
) -> Result<(), CommandError> {
    run_check_command_with_terminal_echo_suppressed(
        root,
        args,
        default_in_place,
        command_persistent_state_root,
        diagnostic_log_plan,
        started,
    )
}

fn run_check_command_with_terminal_echo_suppressed(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    command_persistent_state_root: Option<crate::state_paths::CanonStateRoot>,
    diagnostic_log_plan: Option<DiagnosticLogPlan>,
    started: Instant,
) -> Result<(), CommandError> {
    let mut public_output_progress = CheckPublicOutputProgress::default();
    let parsed_command = parse_check_command_args(args, default_in_place);
    let in_place = parsed_command
        .as_ref()
        .map(|command| command.in_place)
        .unwrap_or_else(|_| preparse_args_use_in_place(args, default_in_place));
    let mut failure_output = requested_check_output(
        started,
        parsed_command
            .as_ref()
            .is_ok_and(check_command_emits_feedback),
    );
    let mut progress_report = CheckRunReport {
        records: Vec::new(),
        cached_passes: Vec::new(),
        pending: 0,
    };
    let mut panic_token_usage = TokenUsageSummary::unavailable();
    // [d] One repository-input snapshot and one prompt-tree OID cache span
    // normal preparation and every default-feedback fallback in this command.
    let mut inspection = CheckCommandInspection::new();
    let command_persistent_state_root = command_persistent_state_root.as_ref();
    let mut diagnostic_log = match start_check_diagnostic_log(
        root,
        command_persistent_state_root,
        diagnostic_log_plan,
    ) {
        Ok(diagnostic_log) => diagnostic_log,
        Err(err) => {
            write_default_failure_output(root, in_place, &mut failure_output, &mut inspection)?;
            return Err(CommandError::from(err));
        }
    };
    let caught_result = catch_unwind(AssertUnwindSafe(|| {
        run_check_command_with_writer(
            root,
            parsed_command,
            in_place,
            command_persistent_state_root,
            started,
            &mut public_output_progress,
            &mut failure_output,
            &mut progress_report,
            &mut panic_token_usage,
            &mut diagnostic_log,
            &mut inspection,
        )
    }));
    match caught_result {
        Ok(result) => {
            let diagnostic_log_error = diagnostic_log.finish_deferred_writes().err();
            finish_check_command(result, diagnostic_log_error)
        }
        Err(payload) => resume_panicked_check(
            payload,
            PanickedCheckContext {
                root,
                in_place,
                public_output_progress,
                failure_output: &mut failure_output,
                progress_report: &progress_report,
                panic_token_usage,
                diagnostic_log: &mut diagnostic_log,
                inspection: &mut inspection,
            },
        ),
    }
}
