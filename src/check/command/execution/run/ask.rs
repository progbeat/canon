use super::super::query::{run_check_query_command, CheckQueryCommand, CheckQueryError};
use crate::check::command::args::parse_ask_command_args;
use crate::check::core::AskCommandArgs;
use crate::check::CheckRunCaches;
use crate::cli::{AskFailure, CommandError};
use crate::config_types::{AgentConfig, CheckConfig, CheckHooksConfig};
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::platform::{install_check_signal_handlers, reset_check_interrupted};
use crate::repo_inspection::RepoInspectionCache;
use std::ffi::OsString;
use std::path::Path;

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
    // xpec: 5
    // "canon ask always asks" starts after parse/tree/log setup accepts the
    // invocation. These resolves validate the optional Git context for a
    // git-backed ask; they are not cache/config shortcuts. Once the command
    // context is valid, config loading falls back and `run_ask_query` always
    // builds the temporary ask xpec before reaching the evaluator boundary.
    let checked_tree = TreeSource::resolve(root, &command.tree, "--tree")?;
    let against_tree = TreeSource::resolve_default_against_tree(
        root,
        &command.against_tree,
        command.against_tree_explicit,
    )?;
    let mut repo_cache = RepoInspectionCache::new();
    let diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let mut check_caches = CheckRunCaches::new();
    let config = ask_query_config_from_optional_check_config(
        repo_cache.load_check_config_with_default_agent_preset(
            root,
            &command.config_path,
            &checked_tree,
            command.default_agent_preset.as_deref(),
        ),
    );
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
    let diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    // xpec: 5
    // In-place ask has no Git-tree preparation. The check config is optional
    // agent context only; load errors fall back to the implementation default
    // agent instead of preventing the temporary ask xpec.
    let config = ask_query_config_from_optional_check_config(
        repo_cache.load_in_place_check_config_with_default_agent_preset(
            root,
            &command.config_path,
            command.default_agent_preset.as_deref(),
        ),
    );
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

fn ask_query_config_from_optional_check_config(config: Result<CheckConfig, String>) -> CheckConfig {
    // `canon ask` always asks the evaluator. A loaded check config is only an
    // optional source of resolved agent settings; check expectations and hooks
    // are discarded before query.rs builds the single temporary ask xpec.
    config
        .map(ask_query_config_from_check_config)
        .unwrap_or_else(|_| ask_query_config_with_agent(AgentConfig::implementation_default()))
}

fn ask_query_config_from_check_config(config: CheckConfig) -> CheckConfig {
    ask_query_config_with_agent(config.agent)
}

fn ask_query_config_with_agent(agent: AgentConfig) -> CheckConfig {
    CheckConfig {
        version: 1,
        agent,
        hooks: CheckHooksConfig::default(),
        expectations: Vec::new(),
    }
}

fn run_ask_query(
    root: &Path,
    command: &AskCommandArgs,
    tree_source: Option<&TreeSource>,
    against_tree: Option<&TreeSource>,
    config: &CheckConfig,
    diagnostic_log: Option<DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    // Ask receives an ask-only `CheckConfig`: agent settings may come from the
    // expanded check config, but configured expectations/hooks are not selected.
    // A prepared ask means parse/tree/log setup has accepted the invocation.
    // After that point there is no cache or last-result shortcut, and the
    // query path always sends an evaluator turn.
    let result = run_check_query_command(CheckQueryCommand {
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
        CheckQueryError::TokenUsage(_) => AskFailure::TokenUsage,
        CheckQueryError::Command(_) | CheckQueryError::Evaluator(_) => AskFailure::Query,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{CheckHookConfig, Expectation};

    #[test] // xpec: 5,HW
    fn ask_query_error_uses_typed_sentinel_command_error() {
        let result =
            ask_query_command_result(Err(CheckQueryError::Evaluator("query failed".to_string())));

        assert_eq!(result, Err(CommandError::AskFailed(AskFailure::Query)));
        assert_eq!(
            ask_query_command_result(Err(CheckQueryError::ReviewRequired("InvalidQuestion"))),
            Err(CommandError::AskFailed(AskFailure::ReviewRequired))
        );
    }

    #[test] // xpec: 5
    fn ask_config_load_error_still_builds_temporary_query_config() {
        let config =
            ask_query_config_from_optional_check_config(Err("config unavailable".to_string()));

        assert!(config.expectations.is_empty());
        assert!(config.hooks.on_start.is_empty());
        assert!(config.hooks.on_pass.is_empty());
    }

    #[test] // xpec: 5
    fn ask_query_config_discards_loaded_check_expectations_and_hooks() {
        let config = ask_query_config_from_optional_check_config(Ok(CheckConfig {
            version: 1,
            agent: AgentConfig::implementation_default(),
            hooks: CheckHooksConfig {
                on_start: vec![CheckHookConfig {
                    print: Some("check-only hook".to_string()),
                    input: None,
                    exec: None,
                    cases: Default::default(),
                }],
                on_pass: Vec::new(),
            },
            expectations: vec![Expectation {
                q: "Does ask ignore configured check expectations?".to_string(),
                a: "yes".to_string(),
                question_context: String::new(),
                diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
                diff_from_configured: true,
                target: None,
                question_answer_only: false,
                agent: AgentConfig::implementation_default(),
                cooldown: None,
            }],
        }));

        assert!(config.expectations.is_empty());
        assert!(config.hooks.on_start.is_empty());
        assert!(config.hooks.on_pass.is_empty());
    }
}
