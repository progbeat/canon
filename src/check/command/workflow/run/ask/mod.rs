use super::root::{git_backed_diagnostic_log_plan, resolve_check_like_root};
use crate::check::command::args::{ask_help_command, parse_ask_command_args};
use crate::check::command::workflow::prepare::resolve_git_backed_tree_state;
use crate::check::command::{
    print_token_usage_summary, GitBackedCheckResources, TokenUsageSummary,
};
use crate::check::core::AskCommandArgs;
use crate::check::interrogation::state::CheckTreeContext;
use crate::check::{load_ask_config, load_in_place_ask_config, CheckRunCaches};
use crate::cli::{print_help_if_requested, CommandError};
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::{DiagnosticLogPlan, DiagnosticLogWriter};
use crate::platform::process::{install_check_signal_handlers, reset_check_interrupted};
use std::ffi::OsString;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::Path;

mod config;
mod error;
mod query;

use config::ask_query_config;
use error::ask_query_command_result;
use query::{run_ask_query_command, AskQueryCommand, AskQueryError};

pub(crate) fn run_ask_command(args: &[OsString]) -> Result<(), CommandError> {
    // xpec: l
    // This is the outermost public `canon ask` boundary. Help rendering,
    // current-directory/root discovery, parsing, preparation, evaluation, and
    // output all run inside the command's token-usage `finally`.
    let mut token_usage_summary = TokenUsageSummary::unavailable();
    let caught_command_result = catch_unwind(AssertUnwindSafe(|| {
        if print_help_if_requested(args, ask_help_command())? {
            return Ok(());
        }
        let command_root = resolve_check_like_root(args)?;
        let diagnostic_log_plan = git_backed_diagnostic_log_plan(&command_root);
        run_prepared_ask_command(
            &command_root.root,
            args,
            command_root.default_in_place,
            diagnostic_log_plan,
            &mut token_usage_summary,
        )
    }));
    let token_usage_result = print_token_usage_summary(token_usage_summary);
    let command_result = match caught_command_result {
        Ok(result) => result,
        // The token-usage finally action has already been attempted. Preserve
        // the original panic instead of converting or masking it with a trailer
        // write failure.
        Err(payload) => resume_unwind(payload),
    };
    match (command_result, token_usage_result) {
        (Err(err), _) => Err(err),
        // [2Z] No earlier output describes a trailer write failure. Keep its
        // diagnostic public even though the CLI's best-effort stderr write can
        // encounter the same underlying stream failure.
        (Ok(()), Err(err)) => Err(CommandError::from(err)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_prepared_ask_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    diagnostic_log_plan: Option<DiagnosticLogPlan>,
    token_usage_summary: &mut TokenUsageSummary,
) -> Result<(), CommandError> {
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_ask_command_args(args, default_in_place)?;
    let mut check_caches = CheckRunCaches::new();
    let config_optional = !command.config_explicit && command.default_agent_preset.is_none();
    // [g2,l,w] Both ask modes produce runtime events through one memory-only
    // temporary-query writer, so neither can reach persistent logging or its
    // coordination state.
    let diagnostic_log = DiagnosticLogWriter::create_temporary_query(diagnostic_log_plan)?;
    let (tree_source, tree_context, resources, loaded_config) = if command.in_place {
        // [90,g2,l] Query working data remains in invocation memory; no xpec
        // result is persisted or exposed to the in-place evaluator context.
        // xpec: l
        // In-place ask has no Git-tree preparation. Config expansion constructs
        // exactly the canonical ask xpec: explicit to/q/a plus omitted fields
        // resolved through the selected presets. Only the default optional
        // config may fall back to implementation defaults.
        let loaded_config = load_in_place_ask_config(
            &mut check_caches.repo_inspection,
            root,
            &command.config_path,
            command.default_agent_preset.as_deref(),
            &command.question,
        );
        (None, None, None, loaded_config)
    } else {
        // xpec: l
        // "canon ask always asks" starts after parse/tree/log setup accepts the
        // invocation. These resolves validate the optional Git context for a
        // git-backed ask; they are not cache/config shortcuts. Config expansion
        // then constructs exactly the canonical ask xpec: explicit to/q/a plus
        // omitted fields resolved through the selected presets. An explicit
        // config or preset makes config loading part of selected behavior, so
        // errors are returned.
        let resources = GitBackedCheckResources::temporary_query(
            root,
            &check_caches.temporary_directory_allocator,
        )?;
        let tree_state = resolve_git_backed_tree_state(
            root,
            &command.tree,
            &command.against_tree,
            &mut check_caches.repo_inspection,
            &resources,
        )?;
        let tree_context = tree_state.check_tree_context(
            root,
            &mut check_caches.visible_tree_oid_cache,
            &resources,
        )?;
        let loaded_config = load_ask_config(
            &mut check_caches.repo_inspection,
            root,
            &command.config_path,
            &tree_state.checked_tree,
            command.default_agent_preset.as_deref(),
            &command.question,
        );
        (
            Some(tree_state.checked_tree),
            Some(tree_context),
            Some(resources),
            loaded_config,
        )
    };
    let config = ask_query_config(loaded_config, config_optional, &command.question)?;
    run_ask_query(
        root,
        &command,
        AskQueryRun {
            tree_source,
            tree_context,
            resources,
            config: &config,
            diagnostic_log: Some(diagnostic_log),
            check_caches: &mut check_caches,
            token_usage_summary,
        },
    )
}

struct AskQueryRun<'a> {
    tree_source: Option<TreeSource>,
    tree_context: Option<CheckTreeContext>,
    resources: Option<GitBackedCheckResources>,
    config: &'a CheckConfig,
    diagnostic_log: Option<DiagnosticLogWriter>,
    check_caches: &'a mut CheckRunCaches,
    token_usage_summary: &'a mut TokenUsageSummary,
}

fn run_ask_query(
    root: &Path,
    command: &AskCommandArgs,
    run: AskQueryRun<'_>,
) -> Result<(), CommandError> {
    // Ask receives an ask-only `CheckConfig` containing its one canonical xpec
    // after preset resolution; configured check xpecs are not selected.
    // A prepared ask means parse/tree/log setup has accepted the invocation.
    // After that point there is no cache or last-result shortcut, and the
    // query path always sends an evaluator turn.
    let result = run_ask_query_command(AskQueryCommand {
        root,
        question: &command.question,
        config: run.config,
        tree_source: run.tree_source,
        tree_context: run.tree_context,
        resources: run.resources,
        in_place: command.in_place,
        no_sandbox: command.no_sandbox,
        diagnostic_log: run.diagnostic_log,
        check_caches: run.check_caches,
        token_usage_summary: run.token_usage_summary,
    });
    ask_query_command_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{AgentConfig, Expectation, DEFAULT_DIFF_FROM};

    #[test] // xpec: l
    fn ask_config_load_error_still_builds_temporary_query_config() {
        let config = ask_query_config(
            Err("config unavailable".to_string()),
            true,
            "Does fallback ask work?",
        )
        .expect("optional config errors fall back");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(config.expectations[0].q, "Does fallback ask work?");
        assert!(config.expectations[0].a.is_empty());
    }

    #[test] // xpec: nK
    fn selected_config_error_is_returned() {
        let error = ask_query_config(
            Err("unknown preset: smart".to_string()),
            false,
            "Does preset ask work?",
        )
        .expect_err("selected config errors must not fall back");

        assert_eq!(error, "unknown preset: smart");
    }

    #[test] // xpec: l,nK,1H
    fn ask_query_config_keeps_resolved_temporary_expectation() {
        let config = ask_query_config(
            Ok(CheckConfig {
                version: 1,
                agent: AgentConfig::implementation_default(),
                expectations: vec![Expectation {
                    to: crate::config_types::ExpectationTo::Agent,
                    rank: 0,
                    q: "Does preset ask work?".to_string(),
                    a: String::new(),
                    question_context: "Use preset context.".to_string(),
                    diff_from: DEFAULT_DIFF_FROM.to_string(),
                    target: None,
                    agent: AgentConfig::implementation_default(),
                    cooldown: None,
                    q_scope: Default::default(),
                    in_place_compatibility: Default::default(),
                }],
            }),
            false,
            "Does preset ask work?",
        )
        .expect("loaded config resolves");

        assert_eq!(config.expectations.len(), 1);
        assert_eq!(
            config.expectations[0].question_context,
            "Use preset context."
        );
    }
}
