use super::failure::{
    fail_check_after_start, fail_check_before_selection, finish_check_error_report,
    started_check_output, CheckErrorReportFinish,
};
use super::hooks::{run_check_hooks, CheckHookOutcome};
use super::prepare::{
    prepare_git_backed_check_execution, GitBackedCheckStorage,
    PrepareGitBackedCheckExecutionOptions,
};
use super::trailer::{
    check_command_writes_agent_message, check_report_passed, write_check_trailer,
    write_check_trailer_with_usage, CompletedCheckRun,
};
mod ask;

pub(crate) use ask::run_ask_command;

use crate::app::LazyAppServerRunner;
use crate::check::command::args::parse_check_command_args;
use crate::check::command::output::{write_stdout_record, SharedCheckOutput};
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::config::validation::{
    check_config_loads_plugins, validate_in_place_check_config,
};
use crate::check::core::{BlockedCheckHook, CheckCommandArgs, CheckRunReport};
use crate::check::interrogation::{state::CheckRuntime, write_check_lifecycle_start_event};
use crate::check::run::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::{run_check_with_runner_and_caches, CheckRunCaches, CheckRunSideEffects};
use crate::cli::CommandError;
use crate::git::TreeSource;
use crate::logs::{write_cache_cleanup_event, DiagnosticLogWriter};
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::xpec_state::{
    active_expectation_ids_from_identities, cleanup_stale_xpec_dirs, XpecStateCache,
};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

// Command execution coordinates CLI parsing, tree/config preparation, and
// final reporting. Per-expectation completion and last-result bookkeeping are
// delegated to the check-run execution layer.
pub(crate) fn run_check_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
) -> Result<(), CommandError> {
    let started = Instant::now();
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_check_command_args(args, default_in_place)?;
    if command.in_place {
        // In-place exits before the Git-backed check path below. That path may
        // read or clean persistent xpec state; in-place may write runtime logs
        // only.
        return run_in_place_check_command(root, &command, started);
    }
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let write_agent_message =
        check_command_writes_agent_message(&command, &checked_tree, &against_tree);
    let mut repo_cache = RepoInspectionCache::new();
    // Runtime-log entry point for `canon check`: `src/logs/writer.rs` resolves
    // `${CANON_STATE_DIR}/logs/0.jsonl` and owns JSONL append/flush/rotation.
    // The check lifecycle, cache, evaluator request/response, thread
    // lifecycle, review, token-usage, and final-result paths below route their
    // events through this writer.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let config = match repo_cache.load_check_config(root, &command.config_path, &checked_tree) {
        Ok(config) => config,
        Err(err) => {
            // The config failed before expectation selection, so the check
            // trailer has no records to summarize while the command error
            // still carries the documented recovery text.
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                started_check_output(started),
                err,
            );
        }
    };
    let identities = match expectation_identities(&config) {
        Ok(identities) => identities,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    let options =
        match resolve_check_options_with_identities(&config, &identities, &command.options) {
            Ok(options) => options,
            Err(err) => {
                return fail_check_before_selection(
                    &mut diagnostic_log,
                    None,
                    false,
                    started_check_output(started),
                    err,
                )
            }
        };
    write_check_lifecycle_start_event(
        &mut diagnostic_log,
        None,
        options
            .candidate_expectations
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let on_start_hook = match run_check_hooks(
        root,
        &config.hooks.on_start,
        "on-start",
        &mut result_output,
        &mut diagnostic_log,
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            return fail_check_after_start(
                &mut diagnostic_log,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    if let CheckHookOutcome::Blocked { repair_instruction } = on_start_hook {
        let completed = blocked_check_run(
            CheckRunReport {
                records: Vec::new(),
                cached: Vec::new(),
                blocked_hooks: Vec::new(),
                skipped: config.expectations.len(),
            },
            repair_instruction,
        );
        return finish_blocked_check(
            Some(&mut diagnostic_log),
            &mut result_output,
            &mut check_caches,
            None,
            &completed,
            started,
        );
    }
    let mut execution = match prepare_git_backed_check_execution(
        root,
        &config,
        PrepareGitBackedCheckExecutionOptions {
            tree_source: &checked_tree,
            against_tree: &against_tree,
            no_sandbox: command.no_sandbox,
            storage: GitBackedCheckStorage::Persistent,
        },
        &mut check_caches.visible_tree_oid,
    ) {
        Ok(execution) => execution,
        Err(err) => {
            return fail_check_after_start(
                &mut diagnostic_log,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    cleanup_cache_dirs(root, &identities, &mut diagnostic_log, started)?;
    let runtime = CheckRuntime::materialized(
        root,
        &execution.staged_view,
        &execution.tree_source,
        execution.tree_context.clone(),
        &config,
        command.no_sandbox,
    );
    let records_result = run_check_with_runner_and_caches(
        runtime,
        &options,
        &mut execution.runner,
        CheckRunSideEffects {
            diagnostic_log: Some(&mut diagnostic_log),
            result_output: Some(&mut result_output),
            live_report_output: Some(shared_output.clone()),
            caches: &mut check_caches,
        },
    );
    let completed = match records_result {
        Ok(report) => CompletedCheckRun {
            report,
            error: None,
            interrupted: false,
        },
        Err(err) => CompletedCheckRun {
            report: *err.report,
            error: Some(err.error),
            interrupted: err.interrupted,
        },
    };
    if completed.error.is_none() && check_report_passed(&completed.report) {
        let on_pass_hook = match run_check_hooks(
            root,
            &config.hooks.on_pass,
            "on-pass",
            &mut result_output,
            &mut diagnostic_log,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                return finish_check_error_after_trailer(CheckErrorAfterTrailerContext {
                    diagnostic_log: &mut diagnostic_log,
                    result_output: &mut result_output,
                    check_caches: &mut check_caches,
                    runner: &mut execution.runner,
                    report: &completed.report,
                    started,
                    error: err,
                })
            }
        };
        if let CheckHookOutcome::Blocked { repair_instruction } = on_pass_hook {
            let completed = blocked_check_run(completed.report, repair_instruction);
            return finish_blocked_check(
                Some(&mut diagnostic_log),
                &mut result_output,
                &mut check_caches,
                Some(&mut execution.runner),
                &completed,
                started,
            );
        }
    }
    finish_completed_check(
        Some(&mut diagnostic_log),
        &mut result_output,
        &mut check_caches,
        &mut execution.runner,
        &completed,
        started,
        write_agent_message,
    )
}

fn blocked_check_run(mut report: CheckRunReport, repair_instruction: String) -> CompletedCheckRun {
    // Hook execution stops at the first blocker, so a completed check run can
    // contain at most one blocked hook outcome.
    report
        .blocked_hooks
        .push(BlockedCheckHook { repair_instruction });
    CompletedCheckRun {
        report,
        error: Some("check hook blocked".to_string()),
        interrupted: false,
    }
}

fn cleanup_cache_dirs(
    root: &Path,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
    started: Instant,
) -> Result<(), CommandError> {
    let xpecs_dir = XpecStateCache::default()
        .xpecs_dir(root)
        .map_err(CommandError::from)?;
    let active_ids = active_expectation_ids_from_identities(identities);
    let cleanup = match cleanup_stale_xpec_dirs(&xpecs_dir, &active_ids) {
        Ok(cleanup) => cleanup,
        Err(err) => {
            return fail_check_after_start(
                diagnostic_log,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    write_cache_cleanup_event(diagnostic_log, cleanup.removed, cleanup.kept)?;
    Ok(())
}

fn run_in_place_check_command(
    root: &Path,
    command: &CheckCommandArgs,
    started: Instant,
) -> Result<(), CommandError> {
    // In-place delegates stay behind their component boundaries: CLI dispatch
    // supplies `root` and `default_in_place`, argument parsing rejects Git-tree
    // and cache controls, repo inspection reads this directory directly, and
    // `CheckRuntime::in_place` owns the evaluator view with no persistent xpec
    // state root. This command path coordinates those interfaces and runs
    // config validation before evaluator work starts.
    let mut repo_cache = RepoInspectionCache::new();
    // In-place uses a fresh in-memory cache bundle only because the shared
    // execution APIs accept cache handles. It still writes runtime logs when
    // logging is enabled, but it does not clean persistent cache directories,
    // and passes an in-place runtime whose lower layers skip xpec reads/writes.
    let mut check_caches = CheckRunCaches::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    // This is the `canon check --in-place` validation boundary. Config load or
    // validation failures return through `fail_check_before_selection`, which
    // owns the check summary, token usage line, and runtime log reporting before
    // hook, selection, or evaluator work can start.
    let config = match repo_cache.load_in_place_check_config_with_default_agent_preset(
        root,
        &command.config_path,
        None,
    ) {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    if let Err(err) = validate_in_place_check_config(&config) {
        return fail_check_before_selection(
            &mut diagnostic_log,
            None,
            false,
            started_check_output(started),
            err,
        );
    }
    let identities = expectation_identities(&config)?;
    let options =
        match resolve_check_options_with_identities(&config, &identities, &command.options) {
            Ok(options) => options,
            Err(err) => {
                return fail_check_before_selection(
                    &mut diagnostic_log,
                    None,
                    false,
                    started_check_output(started),
                    err,
                )
            }
        };
    write_check_lifecycle_start_event(
        &mut diagnostic_log,
        None,
        options
            .candidate_expectations
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let on_start_hook = match run_check_hooks(
        root,
        &config.hooks.on_start,
        "on-start",
        &mut result_output,
        &mut diagnostic_log,
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            return fail_check_after_start(
                &mut diagnostic_log,
                false,
                started_check_output(started),
                err,
            )
        }
    };
    if let CheckHookOutcome::Blocked { repair_instruction } = on_start_hook {
        let completed = blocked_check_run(
            CheckRunReport {
                records: Vec::new(),
                cached: Vec::new(),
                blocked_hooks: Vec::new(),
                skipped: config.expectations.len(),
            },
            repair_instruction,
        );
        return finish_blocked_check(
            Some(&mut diagnostic_log),
            &mut result_output,
            &mut check_caches,
            None,
            &completed,
            started,
        );
    }
    let mut runner = LazyAppServerRunner::new_in_place(
        root,
        check_config_loads_plugins(&config),
        &config.agent,
        command.no_sandbox,
    )?;
    let runtime = CheckRuntime::in_place(root, &config, command.no_sandbox);
    // The in-place runtime makes `run_check_with_runner_and_caches` build a
    // direct Evaluate-only work queue: no Git-backed cache selection or xpec
    // ordering is read. The completed records are returned in this invocation's
    // CheckRunReport; the runtime exposes no persistent check-state root for
    // xpec last-result or live-report files.
    let records_result = run_check_with_runner_and_caches(
        runtime,
        &options,
        &mut runner,
        CheckRunSideEffects {
            diagnostic_log: Some(&mut diagnostic_log),
            result_output: Some(&mut result_output),
            live_report_output: Some(shared_output.clone()),
            caches: &mut check_caches,
        },
    );
    let completed = match records_result {
        Ok(report) => CompletedCheckRun {
            report,
            error: None,
            interrupted: false,
        },
        Err(err) => CompletedCheckRun {
            report: *err.report,
            error: Some(err.error),
            interrupted: err.interrupted,
        },
    };
    if completed.error.is_none() && check_report_passed(&completed.report) {
        let on_pass_hook = match run_check_hooks(
            root,
            &config.hooks.on_pass,
            "on-pass",
            &mut result_output,
            &mut diagnostic_log,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                return finish_check_error_after_trailer(CheckErrorAfterTrailerContext {
                    diagnostic_log: &mut diagnostic_log,
                    result_output: &mut result_output,
                    check_caches: &mut check_caches,
                    runner: &mut runner,
                    report: &completed.report,
                    started,
                    error: err,
                })
            }
        };
        if let CheckHookOutcome::Blocked { repair_instruction } = on_pass_hook {
            let completed = blocked_check_run(completed.report, repair_instruction);
            return finish_blocked_check(
                Some(&mut diagnostic_log),
                &mut result_output,
                &mut check_caches,
                Some(&mut runner),
                &completed,
                started,
            );
        }
    }
    finish_completed_check(
        Some(&mut diagnostic_log),
        &mut result_output,
        &mut check_caches,
        &mut runner,
        &completed,
        started,
        false,
    )
}

struct CheckErrorAfterTrailerContext<'a> {
    diagnostic_log: &'a mut DiagnosticLogWriter,
    result_output: &'a mut dyn Write,
    check_caches: &'a mut CheckRunCaches,
    runner: &'a mut crate::app::LazyAppServerRunner,
    report: &'a CheckRunReport,
    started: Instant,
    error: String,
}

fn finish_check_error_after_trailer(
    context: CheckErrorAfterTrailerContext<'_>,
) -> Result<(), CommandError> {
    let CheckErrorAfterTrailerContext {
        diagnostic_log,
        result_output,
        check_caches,
        runner,
        report,
        started,
        error,
    } = context;
    if let Err(err) = write_check_trailer(runner, result_output, report, started) {
        return finish_check_error_report(CheckErrorReportFinish {
            diagnostic_log,
            result_output,
            check_caches,
            report,
            error: err,
            write_token_usage: false,
        });
    }
    finish_check_error_report(CheckErrorReportFinish {
        diagnostic_log,
        result_output,
        check_caches,
        report,
        error,
        write_token_usage: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_completed_check(
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: &mut dyn Write,
    check_caches: &mut CheckRunCaches,
    runner: &mut crate::app::LazyAppServerRunner,
    completed: &CompletedCheckRun,
    started: Instant,
    write_agent_message: bool,
) -> Result<(), CommandError> {
    if let Err(err) = result_output.flush() {
        let err = format!("failed to flush check result to stdout: {}", err);
        let Some(diagnostic_log) = diagnostic_log.as_deref_mut() else {
            return Err(CommandError::from(err));
        };
        return finish_check_error_report(CheckErrorReportFinish {
            diagnostic_log,
            result_output,
            check_caches,
            report: &completed.report,
            error: err,
            write_token_usage: true,
        });
    }
    if let Err(err) = write_check_trailer(runner, result_output, &completed.report, started) {
        let Some(diagnostic_log) = diagnostic_log.as_deref_mut() else {
            return Err(CommandError::from(err));
        };
        return finish_check_error_report(CheckErrorReportFinish {
            diagnostic_log,
            result_output,
            check_caches,
            report: &completed.report,
            error: err,
            write_token_usage: false,
        });
    }
    let completed_error = completed.error.clone();
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log,
            result_output,
            check_caches,
            // Agent messages are specified for completed default runs. A
            // resource/control interruption can leave pending expectations
            // after a visible result, so it is reported without an instruction.
            write_agent_message: write_agent_message && !completed.interrupted,
        },
        &completed.report,
        completed_error.as_deref(),
    )?;
    if completed.error.is_none() && check_report_passed(&completed.report) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_blocked_check(
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: &mut dyn Write,
    check_caches: &mut CheckRunCaches,
    runner: Option<&mut crate::app::LazyAppServerRunner>,
    completed: &CompletedCheckRun,
    started: Instant,
) -> Result<(), CommandError> {
    match runner {
        Some(runner) => write_check_trailer(runner, result_output, &completed.report, started)?,
        None => write_check_trailer_with_usage(result_output, &completed.report, started, None)?,
    }
    write_blocked_repair_instruction(result_output, &completed.report)?;
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log,
            result_output,
            check_caches,
            write_agent_message: false,
        },
        &completed.report,
        completed.error.as_deref(),
    )?;
    Err(CommandError::CheckFailed)
}

fn write_blocked_repair_instruction(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
) -> Result<(), String> {
    let Some(blocked) = report.blocked_hooks.first() else {
        return Ok(());
    };
    write_stdout_record(
        result_output,
        repair_instruction_line(&blocked.repair_instruction).as_bytes(),
        "check hook repair instruction",
    )
}

fn repair_instruction_line(instruction: &str) -> String {
    if instruction.ends_with('\n') {
        return instruction.to_string();
    }
    let mut line = instruction.to_string();
    line.push('\n');
    line
}
