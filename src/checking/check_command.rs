use crate::app_server::LazyAppServerRunner;
use crate::check::{run_check_with_runner_and_caches, CheckRunCaches};
use crate::check_command_args::parse_check_command_args;
use crate::check_command_finish::{finish_check_report, CheckReportFinishContext};
use crate::check_interrogation_state::CheckRuntime;
use crate::check_lazy_reset::apply_scheduled_lazy_full_scope_resets;
use crate::check_output::write_summary_line;
use crate::check_query_command::run_check_query_command;
use crate::check_reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
};
use crate::check_selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check_types::{CheckRecord, CheckRunReport};
use crate::check_validation::check_config_loads_plugins;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::history_cleanup::{active_expectation_ids_from_identities, cleanup_stale_cache_dirs};
use crate::logging::DiagnosticLogWriter;
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::staged_worktree::StagedWorktreeView;
use crate::visible_tree_oid::VisibleTreeOidCache;
use crate::CANON_CACHE_DIR_GIT_PATH;
use serde_json::json;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let write_agent_message = check_command_writes_agent_message(args);
    let command = parse_check_command_args(args)?;
    let mut repo_cache = RepoInspectionCache::new();
    // Runtime logs are canon-owned state under `${CANON_STATE_DIR}/logs`, not
    // project working-tree content. They are created before snapshot evaluation
    // and are denied to evaluator sessions by the mandatory ignore policy.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let config = match repo_cache.load_check_config(root, &command.config_path) {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                Some(command.query.is_some()),
                command.query.is_some(),
                1,
                err,
            )
        }
    };
    let query_mode = command.query.is_some();
    let query_start_field = if query_mode { Some(true) } else { None };
    // Scheduled lazy resets take effect at the beginning of the next
    // `canon check` invocation, including query-mode invocations that return
    // before normal expectation selection and evaluation. Normal expectation
    // checks also plan the next lazy reset at the end of this command through
    // `check_command_finish::finish_check_report`.
    let identities = match expectation_identities(&config) {
        Ok(identities) => identities,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                query_start_field,
                query_mode,
                0,
                err,
            )
        }
    };
    if let Err(err) = apply_scheduled_lazy_full_scope_resets(root, &config, &identities) {
        return fail_check_before_selection(
            &mut diagnostic_log,
            query_start_field,
            query_mode,
            0,
            err,
        );
    }
    if let Some(question) = command.query.as_deref() {
        return run_check_query_command(
            root,
            &config,
            question,
            &command.query_scope,
            diagnostic_log,
        )
        .map_err(CommandError::from);
    }
    // Check-specific options are parsed with the active config so selectors can
    // be resolved against expectation IDs.
    let options =
        match resolve_check_options_with_identities(&config, &identities, &command.options) {
            Ok(options) => options,
            Err(err) => {
                return fail_check_before_selection(&mut diagnostic_log, None, false, 0, err)
            }
        };
    write_check_start_event(
        &mut diagnostic_log,
        None,
        options
            .selected
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
    )?;
    let mut check_caches = CheckRunCaches::new();
    let mut execution = prepare_check_execution(
        root,
        &config,
        &mut diagnostic_log,
        false,
        0,
        &mut check_caches.visible_tree_oid,
    )
    .map_err(CommandError::from)?;
    let cache_dir = repo_cache
        .git_path(root, CANON_CACHE_DIR_GIT_PATH)
        .map_err(CommandError::from)?;
    let active_ids = active_expectation_ids_from_identities(&identities);
    let cleanup = match cleanup_stale_cache_dirs(&cache_dir, &active_ids) {
        Ok(cleanup) => cleanup,
        Err(err) => return fail_check_after_start(&mut diagnostic_log, false, 1, err),
    };
    if cleanup.sampled {
        diagnostic_log.write_event(
            "info",
            "cache.cleanup",
            &[
                ("removed", json!(cleanup.removed)),
                ("kept", json!(cleanup.kept)),
            ],
        )?;
    }
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut result_output: &mut dyn Write = &mut stdout;
    // `run_check_with_runner` calls `write_and_flush_result_output` after each
    // selected expectation; that helper renders the public human-readable
    // check-output record (`P. OK`, `P. FAILED`, or `P. ERROR`) and flushes it
    // before the next expectation starts.
    let runtime = CheckRuntime::materialized(root, &execution.staged_view, &config);
    // This expectation loop computes the final `CheckRunReport`. It writes and
    // flushes each per-expectation stdout record inside the loop; the public
    // trailer does not exist until the report and final token usage exist.
    let records_result = run_check_with_runner_and_caches(
        runtime,
        &options,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
        &mut check_caches,
    );
    if let Err(err) = result_output.flush() {
        let err = format!("failed to flush check result to stdout: {}", err);
        return fail_check_after_start(&mut diagnostic_log, false, 1, err);
    }
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
    if let Err(err) = write_check_trailer(
        &mut execution.runner,
        &mut *result_output,
        &completed.report,
        started,
    ) {
        return fail_check_after_start(&mut diagnostic_log, false, 1, err);
    }
    finish_check_report(
        CheckReportFinishContext {
            root,
            config: &config,
            diagnostic_log: &mut diagnostic_log,
            result_output: &mut *result_output,
            check_caches: &mut check_caches,
            write_agent_message,
        },
        &completed.report,
        completed.error.as_deref(),
    )?;
    if completed.error.is_none() && completed.report.records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}

pub(crate) fn check_command_writes_agent_message(args: &[OsString]) -> bool {
    args.is_empty()
}

struct CompletedCheckRun {
    report: CheckRunReport,
    error: Option<String>,
}

fn write_check_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    selected: Vec<String>,
) -> Result<(), CommandError> {
    let mut fields = Vec::new();
    if let Some(query) = query {
        fields.push(("query", json!(query)));
    }
    fields.push(("selected", json!(selected)));
    diagnostic_log
        .write_event("info", "check.start", &fields)
        .map_err(CommandError::from)
}

fn fail_check_before_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    start_query: Option<bool>,
    finish_query: bool,
    errors: usize,
    err: String,
) -> Result<(), CommandError> {
    write_check_start_event(diagnostic_log, start_query, Vec::new())?;
    fail_check_after_start(diagnostic_log, finish_query, errors, err)
}

fn fail_check_after_start(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors: usize,
    err: String,
) -> Result<(), CommandError> {
    // Keep the finish-event writer stringly-typed so preflight setup failures
    // can share it without converting their own Result type through CommandError.
    write_check_error_finish_event(diagnostic_log, query, errors, &err)
        .map_err(CommandError::from)?;
    Err(err.into())
}

fn write_check_error_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    _errors: usize,
    err: &str,
) -> Result<(), String> {
    write_check_finish_event(diagnostic_log, query, Some(err))
}

pub(crate) struct PreparedCheckExecution {
    pub(crate) staged_view: StagedWorktreeView,
    pub(crate) runner: LazyAppServerRunner,
}

fn write_check_trailer(
    runner: &mut LazyAppServerRunner,
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
) -> Result<(), String> {
    let usage = collect_check_token_usage(runner)?;
    // The check-output spec orders trailer pieces as token usage, then summary.
    // Token usage is not known until pending app-server usage updates are
    // drained here; once known, each trailer line is rendered, written, and
    // flushed immediately in that order.
    print_token_usage_summary(Some(usage))?;
    write_summary_line(result_output, report, started.elapsed())
}

pub(crate) fn prepare_check_execution(
    root: &Path,
    config: &CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<PreparedCheckExecution, String> {
    // Prepare a scope materializer outside the real working tree so evaluator
    // sessions cannot observe unstaged, untracked, or non-visible project
    // content.
    let staged_view =
        match StagedWorktreeView::apply_with_visible_tree_oid_cache(root, visible_tree_oid_cache) {
            Ok(staged_view) => staged_view,
            Err(err) => {
                write_prepare_check_failure(diagnostic_log, query, errors_on_failure, &err)?;
                return Err(err);
            }
        };
    // The app-server starts from the real project root so Canon-owned runtime
    // state and model catalog config stay under that repository's `.git/canon`.
    // Evaluator sessions get a materialized visible tree as `thread/start.cwd` in
    // `check_interrogation::start_thread_session`.
    let runner = LazyAppServerRunner::new(root, check_config_loads_plugins(config), &config.agent);
    Ok(PreparedCheckExecution {
        staged_view,
        runner,
    })
}

fn write_prepare_check_failure(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
    err: &str,
) -> Result<(), String> {
    write_check_error_finish_event(diagnostic_log, query, errors_on_failure, err)
}
