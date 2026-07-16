use crate::app::LazyAppServerRunner;
use crate::check::command::output::{summary_outcome_counts, write_summary_line};
use crate::check::command::{collect_check_token_usage, print_token_usage_summary};
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CHECK_PATH;
use crate::git::{TreeSource, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
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
    counts.failed == 0 && report.skipped == 0
}

pub(super) fn check_command_writes_agent_message(
    command: &CheckCommandArgs,
    checked_tree: &TreeSource,
    against_tree: &TreeSource,
) -> bool {
    !command.in_place
        && check_config_path_is_default(&command.config_path)
        && command.tree == STAGED_TREE_ARG
        && command.against_tree == DEFAULT_AGAINST_TREE_ARG
        && checked_tree.is_default_checked_tree()
        && against_tree.is_default_against_tree()
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
    print_token_usage_summary(usage)?;
    write_summary_line(result_output, report, started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::RawCheckOptions;
    use std::path::PathBuf;

    #[test] // xpec: v1
    fn agent_message_allows_normalized_default_config_path() {
        let command = command_with_config_path("./.canon/check.yml");

        assert!(check_command_writes_agent_message(
            &command,
            &TreeSource::Staged,
            &default_against_tree()
        ));
    }

    #[test] // xpec: v1
    fn agent_message_rejects_non_default_config_path() {
        let command = command_with_config_path(".canon/other.yml");

        assert!(!check_command_writes_agent_message(
            &command,
            &TreeSource::Staged,
            &default_against_tree()
        ));
    }

    #[test] // xpec: v1
    fn agent_message_allows_explicit_selectors_with_default_command_sources() {
        let mut command = command_with_config_path(".canon/check.yml");
        command.options.selectors.push("a7F".into());

        assert!(check_command_writes_agent_message(
            &command,
            &TreeSource::Staged,
            &default_against_tree()
        ));
    }

    // xpec: T
    #[test]
    fn check_report_passed_rejects_pending_expectations() {
        let report = CheckRunReport {
            records: Vec::new(),
            cached: Vec::new(),
            skipped: 1,
        };

        // xpec: T
        assert!(!check_report_passed(&report));
    }

    fn command_with_config_path(config_path: &str) -> CheckCommandArgs {
        CheckCommandArgs {
            config_path: PathBuf::from(config_path),
            tree: crate::git::STAGED_TREE_ARG.to_string(),
            against_tree: DEFAULT_AGAINST_TREE_ARG.to_string(),
            against_tree_explicit: false,
            in_place: false,
            no_sandbox: false,
            options: RawCheckOptions::default(),
        }
    }

    fn default_against_tree() -> TreeSource {
        TreeSource::Git {
            treeish: DEFAULT_AGAINST_TREE_ARG.to_string(),
            tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        }
    }
}
