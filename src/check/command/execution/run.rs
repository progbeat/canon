use super::failure::{
    fail_check_after_start, fail_check_before_selection, finish_check_error_report,
    CheckErrorReportFinish,
};
use super::prepare::{prepare_check_execution, PrepareCheckExecutionOptions};
use super::query_preset::check_config_with_query_preset;
use super::trailer::{
    check_command_writes_agent_message, check_report_passed, write_check_trailer, CompletedCheckRun,
};
use crate::check::command::args::parse_check_command_args;
use crate::check::command::finish::{finish_check_report, CheckReportFinishContext};
use crate::check::command::output::SharedCheckOutput;
use crate::check::command::query::{run_check_query_command, CheckQueryCommand};
use crate::check::core::types::CheckCommandArgs;
use crate::check::interrogation::{state::CheckRuntime, write_check_lifecycle_start_event};
use crate::check::run::lazy_reset::{
    active_lazy_full_scope_reset_ids, apply_lazy_full_scope_reset,
    clear_evaluated_lazy_full_scope_resets,
};
use crate::check::run::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::{run_check_with_runner_and_caches, CheckRunCaches, CheckRunSideEffects};
use crate::cli::CommandError;
use crate::git::TreeSource;
use crate::history::{active_expectation_ids_from_identities, cleanup_stale_cache_dirs};
use crate::logs::{write_cache_cleanup_event, DiagnosticLogWriter};
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::state_paths::CANON_CACHE_DIR_GIT_PATH;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_check_command_args(args)?;
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let write_agent_message = check_command_writes_agent_message(
        &command.config_path,
        &checked_tree,
        &against_tree,
        !command.options.selectors.is_empty(),
    );
    let mut repo_cache = RepoInspectionCache::new();
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
    let active_reset_ids =
        match active_lazy_full_scope_reset_ids(root, &identities, &mut check_caches.lazy_reset) {
            Ok(ids) => ids,
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
    cleanup_cache_dirs(root, &mut repo_cache, &identities, &mut diagnostic_log)?;
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
        &active_reset_ids,
        &mut execution.runner,
        CheckRunSideEffects {
            diagnostic_log: Some(&mut diagnostic_log),
            result_output: Some(&mut result_output),
            progress_output: Some(shared_output.clone()),
            started,
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
    finish_completed_check(
        root,
        &config,
        &mut diagnostic_log,
        &mut result_output,
        &mut check_caches,
        &mut execution.runner,
        &completed,
        started,
        &active_reset_ids,
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
        tree_source: checked_tree,
        against_tree,
        no_sandbox: command.no_sandbox,
        diagnostic_log,
        check_caches,
    })
    .map_err(CommandError::from)
}

fn cleanup_cache_dirs(
    root: &Path,
    repo_cache: &mut RepoInspectionCache,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), CommandError> {
    let cache_dir = repo_cache
        .git_path(root, CANON_CACHE_DIR_GIT_PATH)
        .map_err(CommandError::from)?;
    let active_ids = active_expectation_ids_from_identities(identities);
    let cleanup = match cleanup_stale_cache_dirs(&cache_dir, &active_ids) {
        Ok(cleanup) => cleanup,
        Err(err) => return fail_check_after_start(diagnostic_log, false, err),
    };
    write_cache_cleanup_event(diagnostic_log, cleanup.removed, cleanup.kept)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_completed_check(
    root: &Path,
    config: &crate::config_types::CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    result_output: &mut dyn Write,
    check_caches: &mut CheckRunCaches,
    runner: &mut crate::app::LazyAppServerRunner,
    completed: &CompletedCheckRun,
    started: Instant,
    active_reset_ids: &std::collections::BTreeSet<String>,
    write_agent_message: bool,
) -> Result<(), CommandError> {
    if let Err(err) = result_output.flush() {
        let err = format!("failed to flush check result to stdout: {}", err);
        return finish_check_error_report(CheckErrorReportFinish {
            root,
            config,
            diagnostic_log,
            result_output,
            check_caches,
            report: &completed.report,
            error: err,
        });
    }
    if let Err(err) = write_check_trailer(runner, result_output, &completed.report, started) {
        return finish_check_error_report(CheckErrorReportFinish {
            root,
            config,
            diagnostic_log,
            result_output,
            check_caches,
            report: &completed.report,
            error: err,
        });
    }
    let mut completed_error = completed.error.clone();
    let mut post_finish_error = None;
    if let Err(err) = clear_evaluated_lazy_full_scope_resets(
        root,
        active_reset_ids,
        &completed.report.records,
        &mut check_caches.lazy_reset,
    ) {
        completed_error.get_or_insert_with(|| err.clone());
        post_finish_error.get_or_insert_with(|| err.into());
    }
    if let Err(err) = apply_lazy_full_scope_reset(
        root,
        config,
        completed.report.evaluated,
        &completed.report.cached,
        &mut check_caches.lazy_reset,
        diagnostic_log,
    ) {
        completed_error.get_or_insert_with(|| err.clone());
        post_finish_error.get_or_insert_with(|| err.into());
    }
    finish_check_report(
        CheckReportFinishContext {
            root,
            config,
            diagnostic_log,
            result_output,
            check_caches,
            write_agent_message,
        },
        &completed.report,
        completed_error.as_deref(),
    )?;
    if let Some(err) = post_finish_error {
        return Err(err);
    }
    if completed.error.is_none() && check_report_passed(&completed.report) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}
