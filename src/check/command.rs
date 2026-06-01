use crate::app::server::LazyAppServerRunner;
use crate::check::command_args::parse_check_command_args;
use crate::check::command_finish::{finish_check_report, CheckReportFinishContext};
use crate::check::interrogation_state::CheckRuntime;
use crate::check::lazy_reset::{
    activate_scheduled_lazy_full_scope_resets, active_lazy_full_scope_reset_ids,
};
use crate::check::output::{summary_outcome_counts, write_summary_line};
use crate::check::query_command::{run_check_query_command, CheckQueryCommand};
use crate::check::reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
};
use crate::check::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::types::CheckRunReport;
use crate::check::validation::check_config_loads_plugins;
use crate::check::{run_check_with_runner_and_caches, CheckRunCaches};
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::git::tree_source::TreeSource;
use crate::git::visible_tree_oid::VisibleTreeOidCache;
use crate::history::cleanup::{active_expectation_ids_from_identities, cleanup_stale_cache_dirs};
use crate::logs::DiagnosticLogWriter;
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::staged::StagedWorktreeView;
use crate::{CANON_CACHE_DIR_GIT_PATH, CHECK_PATH};
use serde_json::json;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_check_command_args(args)?;
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = if command.against_tree_explicit {
        TreeSource::resolve(root, &command.against_tree, "--against-tree")?
    } else {
        TreeSource::Git {
            treeish: command.against_tree.clone(),
            tree_oid: String::new(),
        }
    };
    let write_agent_message = check_command_writes_agent_message(
        &command.config_path,
        &checked_tree,
        &against_tree,
        !command.options.selectors.is_empty(),
    );
    let mut repo_cache = RepoInspectionCache::new();
    // Runtime logs are canon-owned state under `${CANON_STATE_DIR}/logs`, not
    // project working-tree content. They are created before snapshot evaluation
    // and are denied to evaluator sessions by the mandatory ignore policy.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let query_mode = command.query.is_some();
    let query_start_field = if query_mode { Some(true) } else { None };
    // Scheduled lazy resets take effect at the beginning of the next
    // `canon check` invocation. Consume the schedule before config-dependent
    // preflight so a broken config cannot indefinitely defer a scheduled reset.
    if let Err(err) = activate_scheduled_lazy_full_scope_resets(root) {
        return fail_check_before_selection(
            &mut diagnostic_log,
            query_start_field,
            query_mode,
            1,
            err,
        );
    }
    let config = match repo_cache.load_check_config(root, &command.config_path, &checked_tree) {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                &mut diagnostic_log,
                query_start_field,
                query_mode,
                1,
                err,
            )
        }
    };
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
    let active_reset_ids = match active_lazy_full_scope_reset_ids(root, &identities) {
        Ok(ids) => ids,
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
    if let Some(question) = command.query.as_deref() {
        let query_config_override;
        let query_config = match command.query_preset.as_deref() {
            Some(preset) => match check_config_with_query_preset(&config, preset) {
                Ok(config) => {
                    query_config_override = config;
                    &query_config_override
                }
                Err(err) => {
                    return fail_check_before_selection(
                        &mut diagnostic_log,
                        query_start_field,
                        query_mode,
                        0,
                        err,
                    )
                }
            },
            None => &config,
        };
        return run_check_query_command(CheckQueryCommand {
            root,
            config: query_config,
            identities: &identities,
            active_lazy_full_scope_reset_ids: &active_reset_ids,
            question,
            query_scope: &command.query_scope,
            tree_source: &checked_tree,
            no_sandbox: command.no_sandbox,
            diagnostic_log,
        })
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
        PrepareCheckExecutionOptions {
            tree_source: &checked_tree,
            no_sandbox: command.no_sandbox,
            query: false,
            errors_on_failure: 0,
        },
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
    let runtime = CheckRuntime::materialized(
        root,
        &execution.staged_view,
        &execution.tree_source,
        &config,
        command.no_sandbox,
    );
    // During the expectation loop, only per-expectation stdout records are
    // eligible for public output; each one is written and flushed inside the
    // loop before unrelated later work starts. The trailer is not eligible
    // until the loop has produced a report and app-server usage can be drained.
    let records_result = run_check_with_runner_and_caches(
        runtime,
        &options,
        &active_reset_ids,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
        &mut check_caches,
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
    if let Err(err) = result_output.flush() {
        let err = format!("failed to flush check result to stdout: {}", err);
        return finish_check_error_report(CheckErrorReportFinish {
            root,
            config: &config,
            diagnostic_log: &mut diagnostic_log,
            result_output: &mut *result_output,
            check_caches: &mut check_caches,
            report: &completed.report,
            error: err,
        });
    }
    if let Err(err) = write_check_trailer(
        &mut execution.runner,
        &mut *result_output,
        &completed.report,
        started,
    ) {
        return finish_check_error_report(CheckErrorReportFinish {
            root,
            config: &config,
            diagnostic_log: &mut diagnostic_log,
            result_output: &mut *result_output,
            check_caches: &mut check_caches,
            report: &completed.report,
            error: err,
        });
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
    if completed.error.is_none() && check_report_passed(&completed.report) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}

fn check_config_with_query_preset(
    config: &CheckConfig,
    preset: &str,
) -> Result<CheckConfig, String> {
    let agent = config
        .presets
        .get(preset)
        .cloned()
        .ok_or_else(|| format!("unknown preset: {}", preset))?;
    let mut query_config = config.clone();
    query_config.agent = agent;
    Ok(query_config)
}

fn check_report_passed(report: &CheckRunReport) -> bool {
    let counts = summary_outcome_counts(report);
    counts.failed == 0 && counts.errors == 0
}

pub(crate) fn check_command_writes_agent_message(
    config_path: &Path,
    checked_tree: &TreeSource,
    against_tree: &TreeSource,
    selectors_provided: bool,
) -> bool {
    !selectors_provided
        && config_path == Path::new(CHECK_PATH)
        && checked_tree.is_default_checked_tree()
        && against_tree.is_default_against_tree()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use std::collections::BTreeMap;

    fn agent(model: &str, thinking: &str) -> AgentConfig {
        AgentConfig {
            models: vec![model.to_string()],
            thinking: thinking.to_string(),
            ignore: Vec::new(),
            plugins: Vec::new(),
        }
    }

    #[test]
    fn query_preset_overrides_default_agent() {
        let default_agent = agent("default-model", "low");
        let smart_agent = agent("smart-model", "high");
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), default_agent.clone());
        presets.insert("smart".to_string(), smart_agent.clone());
        let config = CheckConfig {
            version: 1,
            presets,
            agent: default_agent.clone(),
            expectations: Vec::new(),
        };

        let query_config = check_config_with_query_preset(&config, "smart").unwrap();

        assert_eq!(query_config.agent, smart_agent);
        assert_eq!(config.agent, default_agent);
    }

    #[test]
    fn query_preset_rejects_unknown_name() {
        let default_agent = agent("default-model", "low");
        let mut presets = BTreeMap::new();
        presets.insert("default".to_string(), default_agent.clone());
        let config = CheckConfig {
            version: 1,
            presets,
            agent: default_agent,
            expectations: Vec::new(),
        };

        let err = check_config_with_query_preset(&config, "missing").unwrap_err();

        assert_eq!(err, "unknown preset: missing");
    }
}

struct CompletedCheckRun {
    report: CheckRunReport,
    error: Option<String>,
}

struct CheckErrorReportFinish<'a, 'b> {
    root: &'a Path,
    config: &'a CheckConfig,
    diagnostic_log: &'b mut DiagnosticLogWriter,
    result_output: &'b mut dyn Write,
    check_caches: &'b mut CheckRunCaches,
    report: &'b CheckRunReport,
    error: String,
}

fn finish_check_error_report(context: CheckErrorReportFinish<'_, '_>) -> Result<(), CommandError> {
    let error = context.error;
    finish_check_report(
        CheckReportFinishContext {
            root: context.root,
            config: context.config,
            diagnostic_log: context.diagnostic_log,
            result_output: context.result_output,
            check_caches: context.check_caches,
            write_agent_message: false,
        },
        context.report,
        Some(&error),
    )?;
    Err(error.into())
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
    pub(crate) tree_source: TreeSource,
    pub(crate) runner: LazyAppServerRunner,
}

pub(crate) struct PrepareCheckExecutionOptions<'a> {
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) no_sandbox: bool,
    pub(crate) query: bool,
    pub(crate) errors_on_failure: usize,
}

fn write_check_trailer(
    runner: &mut LazyAppServerRunner,
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
) -> Result<(), String> {
    let usage = collect_check_token_usage(runner)?;
    // The check-output spec orders trailer pieces as token usage, then summary.
    // The stderr usage line is not eligible before pending app-server usage
    // updates are drained here; once known, each trailer line is rendered,
    // written, and flushed immediately in that order.
    print_token_usage_summary(Some(usage))?;
    write_summary_line(result_output, report, started.elapsed())
}

pub(crate) fn prepare_check_execution(
    root: &Path,
    config: &CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    options: PrepareCheckExecutionOptions<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<PreparedCheckExecution, String> {
    // Prepare a scope materializer outside the real working tree so evaluator
    // sessions cannot observe unstaged, untracked, or non-visible project
    // content.
    let staged_view = match StagedWorktreeView::apply_for_tree_source(
        root,
        options.tree_source.clone(),
        visible_tree_oid_cache,
    ) {
        Ok(staged_view) => staged_view,
        Err(err) => {
            write_prepare_check_failure(
                diagnostic_log,
                options.query,
                options.errors_on_failure,
                &err,
            )?;
            return Err(err);
        }
    };
    // The app-server starts from the real project root so Canon-owned runtime
    // state and model catalog config stay under that repository's `.git/canon`.
    // Evaluator sessions get a materialized visible tree as `thread/start.cwd` in
    // `check_interrogation::start_thread_session`.
    let runner = LazyAppServerRunner::new(
        root,
        check_config_loads_plugins(config),
        &config.agent,
        options.no_sandbox,
    );
    Ok(PreparedCheckExecution {
        staged_view,
        tree_source: options.tree_source.clone(),
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
