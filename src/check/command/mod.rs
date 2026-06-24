// The check command owns the `canon check` public contract. `args` renders
// the clap help surface; `output::record`, `output::usage`, and
// `output::summary` render expectation result entries, token usage, summary
// lines, and agent messages. `execution` and `completion` orchestrate when
// those pieces are emitted and flushed.
pub(super) mod args;
mod completion;
mod execution;
pub(super) mod output;
pub(super) mod preflight;

pub(crate) use args::check_help_command;
pub(super) use completion::{
    collect_check_token_usage, finish_check_report, print_token_usage_summary,
    CheckReportFinishContext,
};
pub(crate) use execution::run_check_command;
pub(super) use execution::{prepare_check_execution, PrepareCheckExecutionOptions};
