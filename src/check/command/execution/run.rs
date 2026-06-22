use super::failure::{
    fail_check_after_start, fail_check_before_selection, finish_check_error_report,
    CheckErrorReportFinish,
};
use super::prepare::{prepare_check_execution, PrepareCheckExecutionOptions};
use super::query::{run_check_query_command, CheckQueryCommand};
use super::query_preset::check_config_with_query_preset;
use super::trailer::{
    check_command_writes_agent_message, check_report_passed, write_check_trailer, CompletedCheckRun,
};
use crate::app::LazyAppServerRunner;
use crate::check::command::args::parse_check_command_args;
use crate::check::command::output::SharedCheckOutput;
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::core::{CheckCommandArgs, SelectedExpectation};
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
    let query_mode = command.query.is_some();
    let query_start_field = if query_mode { Some(true) } else { None };
    let mut check_caches = CheckRunCaches::new();
    let config = match repo_cache.load_check_config(root, &command.config_path, &checked_tree) {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                query_start_field,
                query_mode,
                err,
            )
        }
    };
    if let Some(question) = command.query.as_deref() {
        return run_query_mode(
            root,
            &command,
            &checked_tree,
            &against_tree,
            &config,
            question,
            diagnostic_log,
            query_start_field,
            query_mode,
            &mut check_caches,
        );
    }
    let identities = match expectation_identities(&config) {
        Ok(identities) => identities,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                query_start_field,
                query_mode,
                err,
            )
        }
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
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
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

#[allow(clippy::too_many_arguments)]
fn run_query_mode(
    root: &Path,
    command: &CheckCommandArgs,
    checked_tree: &TreeSource,
    against_tree: &TreeSource,
    config: &crate::config_types::CheckConfig,
    question: &str,
    diagnostic_log: DiagnosticLogWriter,
    query_start_field: Option<bool>,
    query_mode: bool,
    check_caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    let query_config_override;
    let query_config = match command.query_preset.as_deref() {
        Some(preset) => match check_config_with_query_preset(config, preset) {
            Ok(config) => {
                query_config_override = config;
                &query_config_override
            }
            Err(err) => {
                let mut diagnostic_log = diagnostic_log;
                return fail_check_before_selection(
                    &mut diagnostic_log,
                    query_start_field,
                    query_mode,
                    err,
                );
            }
        },
        None => config,
    };
    run_check_query_command(CheckQueryCommand {
        root,
        config: query_config,
        question,
        query_scope: &command.query_scope,
        query_scope_provided: command.query_scope_provided,
        tree_source: Some(checked_tree),
        against_tree: Some(against_tree),
        no_sandbox: command.no_sandbox,
        in_place: false,
        diagnostic_log: Some(diagnostic_log),
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
    let mut repo_cache = RepoInspectionCache::new();
    let mut check_caches = CheckRunCaches::new();
    let config = repo_cache.load_in_place_check_config(root, &command.config_path)?;
    if let Some(question) = command.query.as_deref() {
        return run_check_query_command(CheckQueryCommand {
            root,
            config: &config,
            question,
            query_scope: &command.query_scope,
            query_scope_provided: command.query_scope_provided,
            tree_source: None,
            against_tree: None,
            no_sandbox: command.no_sandbox,
            in_place: true,
            diagnostic_log: None,
            check_caches: &mut check_caches,
        })
        .map_err(CommandError::from);
    }
    let identities = expectation_identities(&config)?;
    let options = resolve_check_options_with_identities(&config, &identities, &command.options)?;
    validate_in_place_selected_expectations(&options.selected)?;
    let mut runner = LazyAppServerRunner::new(
        root,
        check_config_loads_plugins(&config),
        &config.agent,
        command.no_sandbox,
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let runtime = CheckRuntime::in_place(root, &config, command.no_sandbox);
    let records_result = run_check_with_runner_and_caches(
        runtime,
        &options,
        &mut runner,
        CheckRunSideEffects {
            diagnostic_log: None,
            result_output: Some(&mut result_output),
            live_report_output: None,
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
    finish_completed_check(
        None,
        &mut result_output,
        &mut check_caches,
        &mut runner,
        &completed,
        started,
        false,
    )
}

fn validate_in_place_selected_expectations(
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    for expectation in expectations {
        let mut invalid = Vec::new();
        if expectation.diff_from != crate::config_types::DEFAULT_DIFF_FROM {
            invalid.push("diff-from");
        }
        if expectation.target.is_some() {
            invalid.push("target");
        }
        if expectation.cooldown.is_some() {
            invalid.push("cooldown");
        }
        if !expectation.agent.ignore.is_empty() {
            invalid.push("ignore");
        }
        if !invalid.is_empty() {
            return Err(format!(
                "{}. ERROR\n{}\nError: invalid-in-place-expectation\nEvidence: selected expectation configures {}",
                expectation.display_id,
                expectation.question,
                invalid.join(", ")
            ));
        }
    }
    Ok(())
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
