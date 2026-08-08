// The check command owns the `canon check` and `canon ask` public contracts.
// `args` renders the clap help surface; `output::record`, `output::feedback`,
// `output::usage`, and `output::summary` render expectation result entries,
// check feedback, token usage, and summary lines. `workflow` and `completion`
// orchestrate when those pieces are emitted and flushed.
pub(super) mod args;
mod completion;
pub(super) mod output;
mod workflow;

pub(crate) use args::{ask_help_command, check_help_command};
pub(super) use completion::print_token_usage_summary;
pub(super) use completion::{
    check_feedback_messages, collect_token_usage_for_summary, finish_check_report,
    run_with_token_usage_panic_capture, CheckReportFinishContext, TokenUsageSummary,
};
pub(super) use workflow::{
    prepare_git_backed_check_execution, resolve_explicit_diff_from_tree_oids,
    GitBackedCheckResources, PrepareGitBackedCheckExecutionOptions,
};
pub(crate) use workflow::{run_ask_command, run_check_command};
