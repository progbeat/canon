use super::super::query::{run_check_query_command, CheckQueryCommand, CheckQueryError};
use crate::check::command::args::parse_ask_command_args;
use crate::check::command::print_token_usage_summary;
use crate::check::core::AskCommandArgs;
use crate::check::{load_ask_config, load_in_place_ask_config, CheckRunCaches};
use crate::cli::{AskFailure, CommandError};
use crate::config_types::{AgentConfig, CheckConfig, Expectation, ExpectationTo};
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use crate::token_usage_types::TokenUsage;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run_ask_command(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
) -> Result<(), CommandError> {
    // xpec: 0N
    // This is the public `canon ask` finally boundary. It emits exactly one
    // usage line after every command attempt, including parse, config, Git,
    // logging, preparation, evaluator, and output failures.
    let mut token_usage = None;
    let command_result =
        run_ask_command_before_token_usage(root, args, default_in_place, &mut token_usage);
    let usage_result = print_token_usage_summary(token_usage);
    match (command_result, usage_result) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(_)) => Err(CommandError::AskFailed(AskFailure::TokenUsage)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_ask_command_before_token_usage(
    root: &Path,
    args: &[OsString],
    default_in_place: bool,
    token_usage: &mut Option<TokenUsage>,
) -> Result<(), CommandError> {
    install_check_signal_handlers().map_err(CommandError::from)?;
    reset_check_interrupted();
    let command = parse_ask_command_args(args, default_in_place)?;
    if command.in_place {
        return run_in_place_ask_command(root, &command, token_usage);
    }
    // xpec: 0N
    // "canon ask always asks" starts after parse/tree/log setup accepts the
    // invocation. These resolves validate the optional Git context for a
    // git-backed ask; they are not cache/config shortcuts. Once the command
    // context is valid, the default optional config may fall back and
    // `run_ask_query` always receives one temporary ask xpec before reaching
    // the evaluator boundary. An explicit config or preset makes config
    // loading part of the command's selected behavior, so errors are returned.
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let mut repo_cache = RepoInspectionCache::new();
    let diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let config_optional = !command.config_explicit && command.default_agent_preset.is_none();
    let config = ask_query_config(
        load_ask_config(
            &mut repo_cache,
            root,
            &command.config_path,
            &checked_tree,
            command.default_agent_preset.as_deref(),
            &command.question,
        ),
        config_optional,
        &command.question,
    )?;
    run_ask_query(
        root,
        &command,
        AskQueryRun {
            tree_source: Some(&checked_tree),
            against_tree: Some(&against_tree),
            config: &config,
            diagnostic_log: Some(diagnostic_log),
            check_caches: &mut check_caches,
            token_usage,
        },
    )
}

fn run_in_place_ask_command(
    root: &Path,
    command: &AskCommandArgs,
    token_usage: &mut Option<TokenUsage>,
) -> Result<(), CommandError> {
    let mut repo_cache = RepoInspectionCache::new();
    let mut check_caches = CheckRunCaches::new();
    let diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    // xpec: 0N
    // In-place ask has no Git-tree preparation. Its selected config and preset
    // still retain their explicit error behavior; only the default optional
    // config may fall back to implementation defaults.
    let config_optional = !command.config_explicit && command.default_agent_preset.is_none();
    let config = ask_query_config(
        load_in_place_ask_config(
            &mut repo_cache,
            root,
            &command.config_path,
            command.default_agent_preset.as_deref(),
            &command.question,
        ),
        config_optional,
        &command.question,
    )?;
    run_ask_query(
        root,
        command,
        AskQueryRun {
            tree_source: None,
            against_tree: None,
            config: &config,
            diagnostic_log: Some(diagnostic_log),
            check_caches: &mut check_caches,
            token_usage,
        },
    )
}

fn ask_query_config(
    config: Result<CheckConfig, String>,
    config_optional: bool,
    question: &str,
) -> Result<CheckConfig, String> {
    // `load_ask_config` has already replaced configured check expectations
    // with the one resolved temporary ask xpec. A user-selected config or
    // preset must resolve successfully; only the command-default config is
    // optional and may fall back to implementation defaults.
    match config {
        Ok(config) => Ok(config),
        Err(err) if !config_optional => Err(err),
        Err(_) => Ok(ask_query_config_with_agent(
            question,
            AgentConfig::implementation_default(),
        )),
    }
}

fn ask_query_config_with_agent(question: &str, agent: AgentConfig) -> CheckConfig {
    CheckConfig {
        version: 1,
        agent: agent.clone(),
        expectations: vec![Expectation {
            to: ExpectationTo::Agent,
            q: question.to_string(),
            a: String::new(),
            rank: 0,
            question_context: String::new(),
            diff_from: None,
            target: None,
            question_answer_only: true,
            agent,
            cooldown: None,
        }],
    }
}

struct AskQueryRun<'a> {
    tree_source: Option<&'a TreeSource>,
    against_tree: Option<&'a TreeSource>,
    config: &'a CheckConfig,
    diagnostic_log: Option<DiagnosticLogWriter>,
    check_caches: &'a mut CheckRunCaches,
    token_usage: &'a mut Option<TokenUsage>,
}

fn run_ask_query(
    root: &Path,
    command: &AskCommandArgs,
    run: AskQueryRun<'_>,
) -> Result<(), CommandError> {
    // Ask receives an ask-only `CheckConfig`: its one temporary xpec carries
    // resolved preset defaults, while configured check expectations are not
    // selected.
    // A prepared ask means parse/tree/log setup has accepted the invocation.
    // After that point there is no cache or last-result shortcut, and the
    // query path always sends an evaluator turn.
    let result = run_check_query_command(CheckQueryCommand {
        root,
        config: run.config,
        query_scope: &command.query_scope,
        query_scope_provided: command.query_scope_provided,
        tree_source: run.tree_source,
        against_tree: run.against_tree,
        no_sandbox: command.no_sandbox,
        in_place: command.in_place,
        diagnostic_log: run.diagnostic_log,
        check_caches: run.check_caches,
        token_usage: run.token_usage,
    });
    ask_query_command_result(result)
}

fn ask_query_command_result(result: Result<(), CheckQueryError>) -> Result<(), CommandError> {
    match result {
        Ok(()) => Ok(()),
        Err(err) => Err(CommandError::AskFailed(ask_failure_for_query_error(&err))),
    }
}

fn ask_failure_for_query_error(err: &CheckQueryError) -> AskFailure {
    match err {
        CheckQueryError::ReviewRequired(_) => AskFailure::ReviewRequired,
        CheckQueryError::Output(_) => AskFailure::Output,
        CheckQueryError::Command(_) | CheckQueryError::Evaluator(_) => AskFailure::Query,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::Expectation;

    #[test] // xpec: 0N
    fn ask_query_error_uses_typed_sentinel_command_error() {
        let result =
            ask_query_command_result(Err(CheckQueryError::Evaluator("query failed".to_string())));

        assert_eq!(result, Err(CommandError::AskFailed(AskFailure::Query)));
        assert_eq!(
            ask_query_command_result(Err(CheckQueryError::ReviewRequired("InvalidQuestion"))),
            Err(CommandError::AskFailed(AskFailure::ReviewRequired))
        );
    }

    #[test] // xpec: 0N
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

    #[test] // xpec: 0N,nK,kP
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
                    diff_from: None,
                    target: None,
                    question_answer_only: false,
                    agent: AgentConfig::implementation_default(),
                    cooldown: None,
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
