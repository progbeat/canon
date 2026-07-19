use super::failure::{
    collected_check_output, fail_check_after_selection, fail_check_before_selection,
    finish_check_error_report, requested_check_output, start_check_or_fail, started_check_output,
    write_required_check_failure_outputs, CheckErrorReportFinish, CheckFailureOutput,
};
use super::prepare::{
    prepare_git_backed_check_execution, resolve_git_backed_check_tree_context,
    GitBackedCheckResources, PrepareGitBackedCheckExecutionOptions,
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
use crate::check::core::{CheckCommandArgs, CheckOptions, RawCheckOptions};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::run::selection::{expectation_identities, resolve_check_options_with_identities};
use crate::check::{
    run_check_with_runner_and_caches, CheckRunCaches, CheckRunSideEffects, ExpectationIdentity,
    CHECK_PATH,
};
use crate::cli::CommandError;
use crate::git::{TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::logs::{write_xpec_state_retention_event, DiagnosticLogPlan, DiagnosticLogWriter};
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::xpec_state::{
    collected_expectation_ids_from_identities, prune_uncollected_xpec_state_dirs,
};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

struct CheckSelection {
    identities: Vec<ExpectationIdentity>,
    options: CheckOptions,
}

struct PreparedCheckRun<'runtime, 'resources> {
    runtime: CheckRuntime<'runtime>,
    options: &'resources CheckOptions,
    runner: &'resources mut LazyAppServerRunner,
    check_caches: &'resources mut CheckRunCaches,
    diagnostic_log: &'resources mut DiagnosticLogWriter,
    started: Instant,
    trailer_attempted: &'resources mut bool,
    write_agent_message: bool,
    need_to_commit: bool,
}

// Command execution coordinates CLI parsing, tree/config preparation, and
// final reporting. Per-expectation completion and last-result bookkeeping are
// delegated to the check-run execution layer.
pub(crate) fn run_check_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    command_persistent_state_root: Option<crate::state_paths::CanonStateRoot>,
    diagnostic_log_plan: DiagnosticLogPlan,
) -> Result<(), CommandError> {
    let started = Instant::now();
    let mut trailer_attempted = false;
    let in_place = preparse_args_use_in_place(args, default_in_place);
    let mut failure_output = requested_check_output(
        started,
        preparse_args_use_default_check_sources(args, default_in_place),
    );
    // [B,7N,cg,g2,hJ] Runtime-event ownership begins before fallible parsing
    // and tree preparation. The already prepared command control-plane value
    // is separate from either mode's checked subject.
    let diagnostic_log = if in_place {
        DiagnosticLogWriter::create_in_place(
            diagnostic_log_plan,
            command_persistent_state_root.as_ref(),
        )
    } else {
        DiagnosticLogWriter::create_from_plan(root, diagnostic_log_plan)
    };
    let mut diagnostic_log = match diagnostic_log {
        Ok(diagnostic_log) => diagnostic_log,
        Err(err) => {
            failure_output = prepare_default_failure_output(root, failure_output, in_place);
            write_required_check_failure_outputs(failure_output)?;
            return Err(CommandError::from(err));
        }
    };
    // [7N] Runtime observability must not interrupt preparation, evaluation, or
    // public finally effects. Every event write remains unconditional; the
    // writer returns its first storage failure after the whole command lifecycle.
    diagnostic_log.defer_write_errors();
    let result = run_check_command_with_writer(
        root,
        args,
        default_in_place,
        command_persistent_state_root.as_ref(),
        started,
        &mut trailer_attempted,
        &mut failure_output,
        &mut diagnostic_log,
    );
    let diagnostic_log_error = diagnostic_log.finish_deferred_writes().err();
    finish_check_command(result, diagnostic_log_error)
}

#[allow(clippy::too_many_arguments)]
fn run_check_command_with_writer(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    command_persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    started: Instant,
    trailer_attempted: &mut bool,
    failure_output: &mut CheckFailureOutput,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), CommandError> {
    let in_place = preparse_args_use_in_place(args, default_in_place);
    if let Err(err) = install_check_signal_handlers() {
        *failure_output = prepare_default_failure_output(root, *failure_output, in_place);
        return fail_check_before_selection(
            diagnostic_log,
            trailer_attempted,
            *failure_output,
            err.to_string(),
        );
    }
    reset_check_interrupted();
    let command = match parse_check_command_args(args, default_in_place) {
        Ok(command) => command,
        Err(err) => {
            *failure_output = prepare_default_failure_output(root, *failure_output, in_place);
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            );
        }
    };
    let result = run_check_command_after_start(
        root,
        &command,
        command_persistent_state_root,
        diagnostic_log,
        started,
        trailer_attempted,
        failure_output,
    );
    if result.is_err() && !*trailer_attempted {
        // [7N,9b] This outer boundary is the `finally` path for failures that
        // occur before config selection or diagnostic logging can own the
        // token/summary/feedback trailer.
        *failure_output = prepare_default_failure_output(root, *failure_output, command.in_place);
        write_required_check_failure_outputs(*failure_output)?;
    }
    result
}

fn finish_check_command(
    result: Result<(), CommandError>,
    diagnostic_log_error: Option<String>,
) -> Result<(), CommandError> {
    match diagnostic_log_error {
        Some(error) => match result {
            Ok(()) => Err(format!("failed to write check runtime log: {error}").into()),
            Err(primary) => {
                Err(format!("{primary}; also failed to write check runtime log: {error}").into())
            }
        },
        None => result,
    }
}

fn preparse_args_use_in_place(args: &[OsString], default_in_place: bool) -> bool {
    default_in_place
        || args
            .iter()
            .take_while(|arg| arg.as_os_str() != OsStr::new("--"))
            .any(|arg| arg == "--in-place")
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
    // [9b] Successful expansion establishes the collected set even when
    // subsequent config validation rejects that set.
    output.with_pending(config.expectation_count())
}

fn prepare_default_failure_output(
    root: &Path,
    output: CheckFailureOutput,
    in_place: bool,
) -> CheckFailureOutput {
    // [I4] In-place failure reporting has no Git-backed fallback: this boundary
    // returns before the default staged config, tree OIDs, or HEAD are read.
    if in_place {
        return output;
    }
    let output = if output.needs_pending_collection() {
        collect_default_pending_failure_output(root, output)
    } else {
        output
    };
    // BeforeCollection may still emit default pending feedback. Only
    // count-derived feedback needs the checked-vs-HEAD commit context below.
    if !output.has_collected_default_feedback_context() {
        return output;
    }
    let checked_tree_oid = match TreeSource::Staged.tree_oid_for_prompt_diff(root) {
        Ok(tree_oid) => tree_oid,
        Err(_) => return output,
    };
    let against_tree =
        match TreeSource::resolve_default_against_tree(root, DEFAULT_AGAINST_TREE_ARG) {
            Ok(tree) => tree,
            Err(_) => return output,
        };
    let against_tree_oid = match against_tree.tree_oid_for_prompt_diff(root) {
        Ok(tree_oid) => tree_oid,
        Err(_) => return output,
    };
    assert_default_feedback_against_head(&against_tree, &against_tree_oid);
    output.with_need_to_commit(checked_tree_oid != against_tree_oid)
}

fn assert_default_feedback_against_head(against_tree: &TreeSource, against_tree_oid: &str) {
    let is_resolved_head = matches!(
        against_tree,
        TreeSource::DefaultAgainstHead { tree_oid } if tree_oid == against_tree_oid
    );
    // xpec: 7N
    assert!(
        is_resolved_head,
        "default-source feedback requires the against tree to be resolved HEAD"
    );
}

// [7N,9b] This fallback is used only when argument parsing or source resolution
// fails before a CheckCommandArgs exists. It determines whether every resolved
// source has its command-default value, including an explicitly supplied value
// equal to that default, so default feedback may remain eligible.
fn preparse_args_use_default_check_sources(args: &[OsString], default_in_place: bool) -> bool {
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

fn resolve_check_selection(
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

fn run_check_command_after_start(
    root: &Path,
    command: &CheckCommandArgs,
    command_persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    diagnostic_log: &mut DiagnosticLogWriter,
    started: Instant,
    trailer_attempted: &mut bool,
    failure_output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    if command.in_place {
        // In-place exits before the Git-backed path below can inspect trees or
        // select cached evaluations. Its dedicated path reads checked contents
        // from the filesystem and separately maintains canon-owned last-result
        // history, bounded state retention, and invocation-local runtime logs.
        return run_in_place_check_command(
            root,
            command,
            command_persistent_state_root,
            diagnostic_log,
            started,
            trailer_attempted,
            failure_output,
        );
    }
    let checked_tree = match TreeSource::resolve(root, &command.tree, "--tree") {
        Ok(tree) => tree,
        Err(err) => {
            *failure_output = prepare_default_failure_output(root, *failure_output, false);
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            );
        }
    };
    let against_tree = match TreeSource::resolve_default_against_tree(root, &command.against_tree) {
        Ok(tree) => tree,
        Err(err) => {
            *failure_output = prepare_default_failure_output(root, *failure_output, false);
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            );
        }
    };
    let write_agent_message = check_command_writes_agent_message(command);
    let mut repo_cache = RepoInspectionCache::new();
    let mut check_caches = CheckRunCaches::new();
    if let Some(command_persistent_state_root) = command_persistent_state_root {
        check_caches
            .xpec_state
            .bind_state_root(root, command_persistent_state_root);
    }
    let resources = GitBackedCheckResources::Persistent;
    let tree_context = match resolve_git_backed_check_tree_context(
        root,
        &checked_tree,
        &against_tree,
        &mut check_caches.visible_tree_oid,
        &resources,
    ) {
        Ok(tree_context) => tree_context,
        Err(err) => {
            *failure_output = prepare_default_failure_output(root, *failure_output, false);
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            );
        }
    };
    let need_to_commit = tree_context.checked_tree_oid != tree_context.against_tree_oid;
    if write_agent_message {
        assert_default_feedback_against_head(&against_tree, &tree_context.against_tree_oid);
        *failure_output = failure_output.with_need_to_commit(need_to_commit);
    }
    let collected_config =
        match collect_check_config(&mut repo_cache, root, &command.config_path, &checked_tree) {
            Ok(config) => config,
            Err(err) => {
                // The config failed before expectation collection. The
                // required summary reports the empty collected outcome domain,
                // while default-source feedback reports that evaluation
                // remains pending without inventing an xpec count.
                return fail_check_before_selection(
                    diagnostic_log,
                    trailer_attempted,
                    *failure_output,
                    err,
                );
            }
        };
    *failure_output = collected_check_output(
        started,
        collected_config.expectation_count(),
        write_agent_message,
    )
    .with_need_to_commit(need_to_commit);
    let config = match collected_config.into_validated() {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let selection = match resolve_check_selection(&config, &command.options) {
        Ok(selection) => selection,
        Err(err) => {
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    start_selected_check(
        &selection,
        diagnostic_log,
        trailer_attempted,
        *failure_output,
        command_persistent_state_root,
    )?;
    let mut execution = match prepare_git_backed_check_execution(
        root,
        &config,
        PrepareGitBackedCheckExecutionOptions {
            tree_source: &checked_tree,
            tree_context,
            no_sandbox: command.no_sandbox,
            resources,
        },
    ) {
        Ok(execution) => execution,
        Err(err) => {
            return fail_check_after_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let runtime = CheckRuntime::materialized(
        root,
        &execution.staged_view,
        &execution.tree_source,
        execution.tree_context.clone(),
        &config,
        command.no_sandbox,
    );
    run_prepared_check(PreparedCheckRun {
        runtime,
        options: &selection.options,
        runner: &mut execution.runner,
        check_caches: &mut check_caches,
        diagnostic_log,
        started,
        trailer_attempted,
        write_agent_message,
        need_to_commit: execution.tree_context.checked_tree_oid
            != execution.tree_context.against_tree_oid,
    })
}

fn start_selected_check(
    selection: &CheckSelection,
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    failure_output: CheckFailureOutput,
    state_root: Option<&crate::state_paths::CanonStateRoot>,
) -> Result<(), CommandError> {
    start_check_or_fail(
        diagnostic_log,
        selection
            .options
            .candidate_expectations
            .iter()
            .map(|expectation| expectation.id.clone())
            .collect(),
        trailer_attempted,
        failure_output,
    )?;
    if let Some(state_root) = state_root {
        enforce_xpec_state_retention(
            state_root,
            &selection.identities,
            diagnostic_log,
            trailer_attempted,
            failure_output,
        )
    } else {
        Ok(())
    }
}

fn enforce_xpec_state_retention(
    state_root: &crate::state_paths::CanonStateRoot,
    identities: &[crate::check::ExpectationIdentity],
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    failure_output: CheckFailureOutput,
) -> Result<(), CommandError> {
    let xpecs_dir = state_root.join("xpecs");
    let collected_ids = collected_expectation_ids_from_identities(identities);
    let retention = match prune_uncollected_xpec_state_dirs(&xpecs_dir, &collected_ids) {
        Ok(retention) => retention,
        Err(err) => {
            return fail_check_after_selection(
                diagnostic_log,
                trailer_attempted,
                failure_output,
                err,
            )
        }
    };
    if let Err(err) =
        write_xpec_state_retention_event(diagnostic_log, retention.removed, retention.kept)
    {
        return fail_check_after_selection(
            diagnostic_log,
            trailer_attempted,
            failure_output,
            err.to_string(),
        );
    }
    Ok(())
}

fn run_in_place_check_command(
    root: &Path,
    command: &CheckCommandArgs,
    command_persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    diagnostic_log: &mut DiagnosticLogWriter,
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
    // [g2,I4] In-place invocation-local caches stay in this fresh in-memory
    // bundle. Separately, status-specific last results are intentional
    // cross-invocation xpec history under CANON_STATE_DIR: they are read for
    // latest-fail ordering and updated without Git-tree fields. They are not
    // invocation-local scratch. Retention of uncollected history is independent
    // of evaluation selection.
    let mut check_caches = CheckRunCaches::new();
    // Config load performs source-independent validation first. The resolved
    // in-place config preserves field presence directly, so mode validation
    // below needs no parallel raw or configured representation.
    let collected_config = match collect_in_place_check_config_with_default_agent_preset(
        &mut repo_cache,
        root,
        &command.config_path,
        None,
    ) {
        Ok(config) => config,
        Err(err) => {
            return fail_check_before_selection(
                diagnostic_log,
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
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    let config = in_place_config.config();
    let selection = match resolve_check_selection(config, &command.options) {
        Ok(selection) => selection,
        Err(err) => {
            return fail_check_before_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err,
            )
        }
    };
    if let Err(err) = in_place_config.validate_configured_fields() {
        return fail_check_before_selection(
            diagnostic_log,
            trailer_attempted,
            *failure_output,
            err,
        );
    }
    if let Some(command_persistent_state_root) = command_persistent_state_root {
        // [1g,I4] Bind the already resolved canon-owned output namespace before
        // evaluation. Its physical location under `.git` by default does not
        // turn status history into Git evaluation information: only status and
        // timestamp are read, and writes omit every Git-tree field.
        check_caches
            .xpec_state
            .bind_state_root(root, command_persistent_state_root);
    }
    let persistent_status_history = command_persistent_state_root.is_some();
    // [fh,g2,I4,Ijl] Status-specific last results are cross-invocation
    // persistent xpec history when the canonical state namespace exists. The
    // shared selected-run start then prunes only uncollected identities,
    // independently of cache or evaluation selection. A non-Git in-place
    // command without CANON_STATE_DIR keeps its current report in memory and
    // has no cross-invocation history to retain.
    start_selected_check(
        &selection,
        diagnostic_log,
        trailer_attempted,
        *failure_output,
        command_persistent_state_root,
    )?;
    let mut runner = match LazyAppServerRunner::new_in_place(
        root,
        check_config_loads_plugins(config),
        &config.agent,
        command.no_sandbox,
    ) {
        Ok(runner) => runner,
        Err(err) => {
            return fail_check_after_selection(
                diagnostic_log,
                trailer_attempted,
                *failure_output,
                err.to_string(),
            )
        }
    };
    let runtime = CheckRuntime::in_place(root, config, persistent_status_history);
    // The in-place runtime makes `run_check_with_runner_and_caches` build a
    // direct Evaluate-only work queue: no Git-backed cached evaluation is
    // selected. The common rank/latest-fail ordering still applies, completed
    // records are returned in this invocation's in-memory CheckRunReport, and a
    // canonical persistent namespace, when available, receives separate
    // status-specific xpec history without Git-tree fields.
    run_prepared_check(PreparedCheckRun {
        runtime,
        options: &selection.options,
        runner: &mut runner,
        check_caches: &mut check_caches,
        diagnostic_log,
        started,
        trailer_attempted,
        write_agent_message: false,
        need_to_commit: false,
    })
}

fn run_prepared_check(context: PreparedCheckRun<'_, '_>) -> Result<(), CommandError> {
    let PreparedCheckRun {
        runtime,
        options,
        runner,
        check_caches,
        diagnostic_log,
        started,
        trailer_attempted,
        write_agent_message,
        need_to_commit,
    } = context;
    let shared_output = SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let records_result = run_check_with_runner_and_caches(
        runtime,
        options,
        runner,
        CheckRunSideEffects {
            diagnostic_log: Some(&mut *diagnostic_log),
            result_output: Some(&mut result_output),
            live_report_output: Some(shared_output),
            caches: check_caches,
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
        Some(diagnostic_log),
        &mut result_output,
        runner,
        &completed,
        started,
        write_agent_message,
        need_to_commit,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_completed_check(
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: &mut dyn Write,
    runner: &mut crate::app::LazyAppServerRunner,
    completed: &CompletedCheckRun,
    started: Instant,
    write_agent_message: bool,
    need_to_commit: bool,
) -> Result<(), CommandError> {
    if let Err(err) = write_check_trailer(runner, result_output, &completed.report, started) {
        let Some(diagnostic_log) = diagnostic_log.as_deref_mut() else {
            return Err(CommandError::from(err));
        };
        return finish_check_error_report(CheckErrorReportFinish {
            diagnostic_log,
            result_output,
            report: &completed.report,
            error: err,
            write_agent_message,
            need_to_commit,
        });
    }
    let completed_error = completed.error.clone();
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log,
            result_output,
            // [AL] This is post-summary feedback, not a success-only message.
            // The `finally` contract emits it for interrupted default-source
            // runs too; report counts select the pending or repair wording.
            write_agent_message,
            need_to_commit,
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

#[cfg(test)]
mod tests {
    use super::{finish_check_command, preparse_args_use_in_place};
    use crate::cli::CommandError;
    use std::ffi::OsString;

    #[test] // xpec: cg
    fn in_place_preparse_stops_at_the_option_separator() {
        assert!(preparse_args_use_in_place(
            &[OsString::from("--in-place")],
            false
        ));
        assert!(!preparse_args_use_in_place(
            &[OsString::from("--"), OsString::from("--in-place")],
            false
        ));
    }

    #[test] // xpec: 7N
    fn deferred_check_log_error_is_returned_after_primary_result() {
        let error = finish_check_command(
            Err(CommandError::CheckFailed),
            Some("sink failed".to_string()),
        )
        .unwrap_err();

        assert_eq!(
            error,
            CommandError::from(
                "canon check failed; also failed to write check runtime log: sink failed"
                    .to_string()
            )
        );
    }
}
