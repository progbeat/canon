use super::failure::{
    fail_check_after_start, fail_check_before_selection, finish_check_error_report,
    CheckErrorReportFinish,
};
use super::hooks::{run_check_hook, CheckHookOutcome};
use super::in_place::{invalid_in_place_expectation_records, validate_in_place_global_config};
use super::prepare::{prepare_check_execution, PrepareCheckExecutionOptions};
use super::query::{run_check_query_command, CheckQueryCommand};
use super::trailer::{
    check_command_writes_agent_message, check_report_passed, write_check_trailer,
    write_check_trailer_with_usage, CompletedCheckRun,
};
use crate::app::LazyAppServerRunner;
use crate::check::command::args::{parse_ask_command_args, parse_check_command_args};
use crate::check::command::output::{
    write_result_output_without_started_report, write_stdout_record, SharedCheckOutput,
};
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::core::{AskCommandArgs, BlockedCheckHook, CheckCommandArgs, CheckRunReport};
use crate::check::interrogation::{state::CheckRuntime, write_check_lifecycle_start_event};
use crate::check::run::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::{
    run_check_with_runner_and_caches, skipped_count, CheckRunCaches, CheckRunSideEffects,
};
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
    // Runtime-log entry point for `canon check`: this writer resolves
    // `${CANON_STATE_DIR}/logs/0.jsonl`, then the check lifecycle, cache,
    // evaluator request/response, thread lifecycle, review, token-usage, and
    // final-result paths below append flushed JSONL events through it.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let config = match repo_cache.load_check_config(root, &command.config_path, &checked_tree) {
        Ok(config) => config,
        Err(err) => return fail_check_before_selection(&mut diagnostic_log, None, false, err),
    };
    let identities = match expectation_identities(&config) {
        Ok(identities) => identities,
        Err(err) => return fail_check_before_selection(&mut diagnostic_log, None, false, err),
    };
    let options =
        match resolve_check_options_with_identities(&config, &identities, &command.options) {
            Ok(options) => options,
            Err(err) => return fail_check_before_selection(&mut diagnostic_log, None, false, err),
        };
    write_check_lifecycle_start_event(
        &mut diagnostic_log,
        None,
        options
            .selected
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let on_start_hook = match run_check_hook(config.hooks.on_start.as_ref(), &mut result_output) {
        Ok(outcome) => outcome,
        Err(err) => return fail_check_after_start(&mut diagnostic_log, false, err),
    };
    if let CheckHookOutcome::Blocked { repair_instruction } = on_start_hook {
        let completed = blocked_check_run(
            CheckRunReport {
                records: Vec::new(),
                cached: Vec::new(),
                blocked: None,
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
    let mut execution = match prepare_check_execution(
        root,
        &config,
        PrepareCheckExecutionOptions {
            tree_source: &checked_tree,
            against_tree: &against_tree,
            no_sandbox: command.no_sandbox,
        },
        &mut check_caches.visible_tree_oid,
    ) {
        Ok(execution) => execution,
        Err(err) => return fail_check_after_start(&mut diagnostic_log, false, err),
    };
    cleanup_cache_dirs(root, &identities, &mut diagnostic_log)?;
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
        let on_pass_hook = match run_check_hook(config.hooks.on_pass.as_ref(), &mut result_output) {
            Ok(outcome) => outcome,
            Err(err) => {
                return finish_check_error_report(CheckErrorReportFinish {
                    diagnostic_log: &mut diagnostic_log,
                    result_output: &mut result_output,
                    check_caches: &mut check_caches,
                    report: &completed.report,
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
    report.blocked = Some(BlockedCheckHook { repair_instruction });
    CompletedCheckRun {
        report,
        error: Some("check hook blocked".to_string()),
        interrupted: false,
    }
}

pub(crate) fn run_ask_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
) -> Result<(), CommandError> {
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_ask_command_args(args, default_in_place)?;
    if command.in_place {
        return run_in_place_ask_command(root, &command);
    }
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let mut repo_cache = RepoInspectionCache::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let config = match repo_cache.load_check_config_with_default_agent_preset(
        root,
        &command.config_path,
        &checked_tree,
        command.default_agent_preset.as_deref(),
    ) {
        Ok(config) => config,
        Err(err) => return fail_check_before_selection(&mut diagnostic_log, Some(true), true, err),
    };
    run_ask_query(
        root,
        &command,
        Some(&checked_tree),
        Some(&against_tree),
        &config,
        Some(diagnostic_log),
        &mut check_caches,
    )
}

fn run_in_place_ask_command(root: &Path, command: &AskCommandArgs) -> Result<(), CommandError> {
    let mut repo_cache = RepoInspectionCache::new();
    let mut check_caches = CheckRunCaches::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let config = match repo_cache.load_in_place_check_config_with_default_agent_preset(
        root,
        &command.config_path,
        command.default_agent_preset.as_deref(),
    ) {
        Ok(config) => config,
        Err(err) => return fail_check_before_selection(&mut diagnostic_log, Some(true), true, err),
    };
    run_ask_query(
        root,
        command,
        None,
        None,
        &config,
        Some(diagnostic_log),
        &mut check_caches,
    )
}

fn run_ask_query(
    root: &Path,
    command: &AskCommandArgs,
    tree_source: Option<&TreeSource>,
    against_tree: Option<&TreeSource>,
    config: &crate::config_types::CheckConfig,
    diagnostic_log: Option<DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    // Ask receives the same already-expanded `CheckConfig` as normal check
    // execution. Preset selection is over by this point; query.rs can only
    // consume fields stored on `CheckConfig` and its expectations.
    run_check_query_command(CheckQueryCommand {
        root,
        config,
        question: &command.question,
        query_scope: &command.query_scope,
        query_scope_provided: command.query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox: command.no_sandbox,
        in_place: command.in_place,
        diagnostic_log,
        check_caches,
    })
    .map_err(CommandError::from)
}

fn cleanup_cache_dirs(
    root: &Path,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), CommandError> {
    let xpecs_dir = XpecStateCache::default()
        .xpecs_dir(root)
        .map_err(CommandError::from)?;
    let active_ids = active_expectation_ids_from_identities(identities);
    let cleanup = match cleanup_stale_xpec_dirs(&xpecs_dir, &active_ids) {
        Ok(cleanup) => cleanup,
        Err(err) => return fail_check_after_start(diagnostic_log, false, err),
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
    // state root. This command path only coordinates those interfaces and
    // rejects selected expectations that use in-place-prohibited fields before
    // evaluator work starts.
    let mut repo_cache = RepoInspectionCache::new();
    // In-place uses a fresh in-memory cache bundle only because the shared
    // execution APIs accept cache handles. It still writes runtime logs when
    // logging is enabled, but it does not clean persistent cache directories,
    // and passes an in-place runtime whose lower layers skip xpec reads/writes.
    let mut check_caches = CheckRunCaches::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let config = match repo_cache.load_in_place_check_config_with_default_agent_preset(
        root,
        &command.config_path,
        None,
    ) {
        Ok(config) => config,
        Err(err) => return fail_check_before_selection(&mut diagnostic_log, None, false, err),
    };
    let identities = expectation_identities(&config)?;
    // This resolves selector/keep-going controls from the expanded config.
    // Persistent state is not consulted here.
    let options =
        match resolve_check_options_with_identities(&config, &identities, &command.options) {
            Ok(options) => options,
            Err(err) => return fail_check_before_selection(&mut diagnostic_log, None, false, err),
        };
    if let Err(err) = validate_in_place_global_config(&config.agent) {
        return fail_check_before_selection(&mut diagnostic_log, None, false, err);
    }
    write_check_lifecycle_start_event(
        &mut diagnostic_log,
        None,
        options
            .selected
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let on_start_hook = match run_check_hook(config.hooks.on_start.as_ref(), &mut result_output) {
        Ok(outcome) => outcome,
        Err(err) => return fail_check_after_start(&mut diagnostic_log, false, err),
    };
    if let CheckHookOutcome::Blocked { repair_instruction } = on_start_hook {
        let completed = blocked_check_run(
            CheckRunReport {
                records: Vec::new(),
                cached: Vec::new(),
                blocked: None,
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
    let invalid_records = invalid_in_place_expectation_records(&config.agent, &options.selected)?;
    if !invalid_records.is_empty() {
        // In-place compatibility errors are result records. Each invalid
        // selected expectation is printed with its short ID before the summary.
        {
            let mut output = Some(&mut result_output as &mut dyn Write);
            for record in &invalid_records {
                write_result_output_without_started_report(&mut output, record)
                    .map_err(CommandError::from)?;
            }
        }
        let records = invalid_records;
        let cached = Vec::new();
        let skipped = skipped_count(config.expectations.len(), &records, &cached);
        let completed = CompletedCheckRun {
            report: CheckRunReport {
                records,
                cached,
                blocked: None,
                skipped,
            },
            error: Some("invalid-in-place-expectation".to_string()),
            interrupted: false,
        };
        return finish_completed_check(
            Some(&mut diagnostic_log),
            &mut result_output,
            &mut check_caches,
            &mut runner,
            &completed,
            started,
            false,
        );
    }
    let runtime = CheckRuntime::in_place(root, &config, command.no_sandbox);
    // The in-place runtime makes `run_check_with_runner_and_caches` build a
    // direct Evaluate-only work queue: no pass snapshot, same-tree cache,
    // cooldown cache, xpec ordering, or cached-result output is read. The
    // completed records are returned in this invocation's
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
        let on_pass_hook = match run_check_hook(config.hooks.on_pass.as_ref(), &mut result_output) {
            Ok(outcome) => outcome,
            Err(err) => {
                return finish_check_error_report(CheckErrorReportFinish {
                    diagnostic_log: &mut diagnostic_log,
                    result_output: &mut result_output,
                    check_caches: &mut check_caches,
                    report: &completed.report,
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
    let Some(blocked) = &report.blocked else {
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
