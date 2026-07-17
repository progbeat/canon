use super::failure::{
    collected_check_output, fail_check_after_start, fail_check_before_selection,
    finish_check_error_report, requested_check_output, started_check_output,
    write_check_failure_trailer, CheckErrorReportFinish, CheckFailureOutput,
};
use super::prepare::{
    prepare_git_backed_check_execution, GitBackedCheckStorage,
    PrepareGitBackedCheckExecutionOptions,
};
use super::trailer::{
    check_command_writes_agent_message, check_config_path_is_default, check_report_passed,
    write_check_trailer, CompletedCheckRun,
};
mod ask;

pub(crate) use ask::run_ask_command;

use crate::app::LazyAppServerRunner;
use crate::check::command::args::parse_check_command_args;
use crate::check::command::output::SharedCheckOutput;
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::config::{
    collect_check_config, collect_in_place_check_config_with_default_agent_preset,
};
use crate::check::core::CheckCommandArgs;
use crate::check::interrogation::{state::CheckRuntime, write_check_lifecycle_start_event};
use crate::check::run::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::{
    run_check_with_runner_and_caches, CheckRunCaches, CheckRunSideEffects, CHECK_PATH,
};
use crate::cli::CommandError;
use crate::git::{TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
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
    let mut trailer_attempted = false;
    let mut failure_output = requested_check_output(
        started,
        preparse_args_use_default_feedback_sources(args, default_in_place),
    );
    let result = run_check_command_after_start(
        root,
        args,
        default_in_place,
        started,
        &mut trailer_attempted,
        &mut failure_output,
    );
    if result.is_err() && !trailer_attempted {
        // This outer boundary is the `finally` path for failures that occur
        // before config selection or diagnostic logging can own the trailer.
        if failure_output.needs_pending_collection() {
            failure_output = collect_default_pending_failure_output(root, failure_output);
        }
        write_check_failure_trailer(failure_output)?;
    }
    result
}

fn collect_default_pending_failure_output(
    root: &Path,
    output: CheckFailureOutput,
) -> CheckFailureOutput {
    let mut repo_cache = RepoInspectionCache::new();
    let Ok(config) = collect_check_config(
        &mut repo_cache,
        root,
        Path::new(CHECK_PATH),
        &TreeSource::Staged,
    ) else {
        return output;
    };
    // [NQ] Successful expansion establishes the collected set even when
    // subsequent config validation rejects that set.
    output.with_pending(config.expectation_count())
}

// [NQ] This fallback is used only when argument parsing or source resolution
// fails before a CheckCommandArgs exists. Once those steps succeed,
// check_command_writes_agent_message computes authoritative eligibility from
// the parsed command-default values and replaces this state.
fn preparse_args_use_default_feedback_sources(args: &[OsString], default_in_place: bool) -> bool {
    if default_in_place {
        return false;
    }
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else {
            continue;
        };
        if arg == "--" {
            break;
        }
        if arg == "--in-place" {
            return false;
        }
        let default_value = match arg {
            "-c" | "--config" => {
                let Some(value) = args.next() else {
                    return false;
                };
                check_config_path_is_default(Path::new(value))
            }
            "--tree" => matches!(
                args.next().and_then(|value| value.to_str()),
                Some(STAGED_TREE_ARG)
            ),
            "--against-tree" => matches!(
                args.next().and_then(|value| value.to_str()),
                Some(DEFAULT_AGAINST_TREE_ARG)
            ),
            _ => match arg.split_once('=') {
                Some(("--config" | "-c", value)) => check_config_path_is_default(Path::new(value)),
                Some(("--tree", value)) => value == STAGED_TREE_ARG,
                Some(("--against-tree", value)) => value == DEFAULT_AGAINST_TREE_ARG,
                _ if arg.starts_with("-c") && arg.len() > 2 => {
                    check_config_path_is_default(Path::new(&arg[2..]))
                }
                _ => continue,
            },
        };
        if !default_value {
            return false;
        }
    }
    true
}

fn run_check_command_after_start(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    started: Instant,
    trailer_attempted: &mut bool,
    failure_output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_check_command_args(args, default_in_place)?;
    if command.in_place {
        // In-place exits before the Git-backed check path below. That path may
        // read or clean persistent xpec state; in-place may write runtime logs
        // only.
        return run_in_place_check_command(
            root,
            &command,
            started,
            trailer_attempted,
            failure_output,
        );
    }
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let write_agent_message = check_command_writes_agent_message(&command);
    let mut repo_cache = RepoInspectionCache::new();
    // Runtime-log entry point for `canon check`: `src/logs/writer.rs` resolves
    // `${CANON_STATE_DIR}/logs/0.jsonl` and owns JSONL append/flush/rotation.
    // The check lifecycle, cache, evaluator request/response, thread
    // lifecycle, review, token-usage, and final-result paths below route their
    // events through this writer.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let collected_config =
        match collect_check_config(&mut repo_cache, root, &command.config_path, &checked_tree) {
            Ok(config) => config,
            Err(err) => {
                // The config failed before expectation selection, so the trailer
                // reports an empty check result before the command error carries
                // its self-contained diagnostic.
                return fail_check_before_selection(
                    &mut diagnostic_log,
                    None,
                    false,
                    trailer_attempted,
                    started_check_output(started, write_agent_message),
                    err,
                );
            }
        };
    *failure_output = collected_check_output(
        started,
        collected_config.expectation_count(),
        write_agent_message,
    );
    let config = match collected_config.into_validated() {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let identities = match expectation_identities(&config) {
        Ok(identities) => identities,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                trailer_attempted,
                *failure_output,
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
                    trailer_attempted,
                    *failure_output,
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
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    cleanup_cache_dirs(
        root,
        &identities,
        &mut diagnostic_log,
        trailer_attempted,
        *failure_output,
    )?;
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
        },
        Err(err) => CompletedCheckRun {
            report: *err.report,
            error: Some(err.error),
        },
    };
    *trailer_attempted = true;
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

fn cleanup_cache_dirs(
    root: &Path,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    failure_output: CheckFailureOutput,
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
                trailer_attempted,
                failure_output,
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
    trailer_attempted: &mut bool,
    failure_output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    // In-place delegates stay behind their component boundaries: CLI dispatch
    // supplies `root` and `default_in_place`, argument parsing rejects Git-tree
    // controls, repo inspection reads this directory directly, and
    // `CheckRuntime::in_place` owns the evaluator view without Git tree state.
    // This command path coordinates those interfaces and runs
    // config validation before evaluator work starts.
    let mut repo_cache = RepoInspectionCache::new();
    // In-place uses a fresh in-memory cache bundle only because the shared
    // execution APIs accept cache handles. In-place evaluation does not reuse
    // cached results, but it writes current last results and removes state for
    // expectations no longer present in the config.
    let mut check_caches = CheckRunCaches::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    // Config load performs source-independent validation first and retains the
    // configured Git-backed-only fields for the in-place mode validation below.
    let collected_config = match collect_in_place_check_config_with_default_agent_preset(
        &mut repo_cache,
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
                trailer_attempted,
                started_check_output(started, false),
                err,
            )
        }
    };
    *failure_output = collected_check_output(started, collected_config.expectation_count(), false);
    let in_place_config = match collected_config.into_validated() {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let config = in_place_config.config();
    let identities = match expectation_identities(config) {
        Ok(identities) => identities,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let options = match resolve_check_options_with_identities(config, &identities, &command.options)
    {
        Ok(options) => options,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                None,
                false,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    if let Err(err) = in_place_config.validate_configured_fields() {
        return fail_check_before_selection(
            &mut diagnostic_log,
            None,
            false,
            trailer_attempted,
            *failure_output,
            err,
        );
    }
    let selected_ids = options
        .candidate_expectations
        .iter()
        .map(|expectation| expectation.id.clone())
        .collect::<Vec<_>>();
    write_check_lifecycle_start_event(&mut diagnostic_log, None, selected_ids)?;
    cleanup_cache_dirs(
        root,
        &identities,
        &mut diagnostic_log,
        trailer_attempted,
        *failure_output,
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let mut runner = LazyAppServerRunner::new_in_place(
        root,
        check_config_loads_plugins(config),
        &config.agent,
        command.no_sandbox,
    )?;
    let runtime = CheckRuntime::in_place(root, config, command.no_sandbox);
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
        },
        Err(err) => CompletedCheckRun {
            report: *err.report,
            error: Some(err.error),
        },
    };
    *trailer_attempted = true;
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
            // [AL] This is post-summary feedback, not a success-only message.
            // The `finally` contract emits it for interrupted default-source
            // runs too; report counts select the pending or repair wording.
            write_agent_message,
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
