use crate::app::LazyAppServerRunner;
use crate::check::command::output::{summary_outcome_counts, write_summary_line};
use crate::check::command::{collect_check_token_usage, print_token_usage_summary};
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CHECK_PATH;
use crate::scope::normalize_repo_path;
use crate::token_usage_types::TokenUsage;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

pub(super) struct CompletedCheckRun {
    pub(super) report: CheckRunReport,
    pub(super) error: Option<String>,
}

pub(super) fn check_report_passed(report: &CheckRunReport) -> bool {
    let counts = summary_outcome_counts(report);
    counts.failed == 0 && counts.pending == 0
}

pub(super) fn check_command_writes_agent_message(command: &CheckCommandArgs) -> bool {
    !command.in_place && command.sources_have_command_default_values
}

pub(super) fn check_config_path_is_default(config_path: &Path) -> bool {
    let Some(config_path) = config_path.to_str() else {
        return false;
    };
    matches!(
        normalize_repo_path(config_path),
        Ok(normalized) if normalized == CHECK_PATH
    )
}

pub(super) fn write_check_trailer(
    runner: &mut LazyAppServerRunner,
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
) -> Result<(), String> {
    let usage = collect_check_token_usage(runner);
    write_check_trailer_with_usage(result_output, report, started, usage)
}

pub(super) fn write_check_trailer_with_usage(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
    usage: Option<TokenUsage>,
) -> Result<(), String> {
    write_required_trailer_parts(
        || print_token_usage_summary(usage),
        || write_summary_line(result_output, report, started.elapsed()),
    )
}

pub(super) fn write_required_trailer_parts(
    write_token_usage: impl FnOnce() -> Result<(), String>,
    write_summary: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // [7N] These are independent `finally` outputs. Attempt both even when
    // stderr rejects the token-usage line, so a failure in one output channel
    // cannot suppress the other required trailer part.
    let token_usage_result = write_token_usage();
    let summary_result = write_summary();
    match (token_usage_result, summary_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(token_error), Err(summary_error)) => Err(format!(
            "{token_error}; also failed to write check summary: {summary_error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::RawCheckOptions;
    use crate::git::DEFAULT_AGAINST_TREE_ARG;
    use std::path::PathBuf;

    #[test] // xpec: 9b
    fn agent_message_allows_normalized_default_config_path() {
        let command = command_with_config_path("./.canon/check.yml");

        assert!(check_command_writes_agent_message(&command));
    }

    #[test] // xpec: 9b
    fn agent_message_rejects_non_default_config_path() {
        let command = command_with_config_path(".canon/other.yml");

        assert!(!check_command_writes_agent_message(&command));
    }

    #[test] // xpec: 9b
    fn agent_message_allows_explicit_selectors_with_default_command_sources() {
        let mut command = command_with_config_path(".canon/check.yml");
        command.options.selectors.push("a7F".into());

        assert!(check_command_writes_agent_message(&command));
    }

    // xpec: T
    #[test]
    fn check_report_passed_rejects_pending_expectations() {
        let report = CheckRunReport {
            records: Vec::new(),
            cached: Vec::new(),
            pending: 1,
        };

        // xpec: T
        assert!(!check_report_passed(&report));
    }

    #[test] // xpec: 7N
    fn trailer_attempts_summary_after_token_usage_failure() {
        let mut token_usage_attempted = false;
        let mut summary_attempted = false;

        let error = write_required_trailer_parts(
            || {
                token_usage_attempted = true;
                Err("token usage failed".to_string())
            },
            || {
                summary_attempted = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(token_usage_attempted);
        assert!(summary_attempted);
        assert_eq!(error, "token usage failed");
    }

    fn command_with_config_path(config_path: &str) -> CheckCommandArgs {
        CheckCommandArgs {
            config_path: PathBuf::from(config_path),
            tree: crate::git::STAGED_TREE_ARG.to_string(),
            against_tree: DEFAULT_AGAINST_TREE_ARG.to_string(),
            sources_have_command_default_values: check_config_path_is_default(Path::new(
                config_path,
            )),
            in_place: false,
            no_sandbox: false,
            options: RawCheckOptions::default(),
        }
    }
}
