// Command output is one component with a small facade. The leaf modules split
// stable stdout/stderr surfaces by output kind while keeping callers away from
// renderer internals.
// `canon check` uses this facade instead of `crate::output` because live
// progress shares stdout across threads. Agent feedback, expectation result
// entries, token usage, and summary output are split across the command
// execution layer plus `feedback`, `record`, `usage`, and `summary`;
// every exported writer flushes through `shared::write_stdout_record` or
// `SharedCheckOutput` as soon as a record, token line, summary, query answer,
// feedback line, or progress dot is eligible.
// [sj] Interactive terminal echo is deliberately not renderer state. The
// command-wide guard lives in `workflow::run::lifecycle::terminal`, and its
// POSIX ECHO/ECHONL operations live in `platform::process`.
// `canon gate` is outside this check-output component because it has no live
// result/progress stream. Its user-visible lines are emitted through
// `crate::output::write_stderr_line` at the validation, error, or regression
// decision points where those lines first become eligible.
mod escape;
mod feedback;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use escape::escape_check_output_text;
pub(crate) use feedback::{
    command_error_feedback_messages, continue_evaluation_message, render_check_feedback_messages,
    CheckFeedbackContext,
};
pub(crate) use query::finish_query_output;
#[cfg(test)]
pub(crate) use record::write_caller_result_output_with_elapsed_timeline;
pub(crate) use record::{
    publish_expectation_report, render_caller_prompt, start_query_report_output,
    write_caller_result_output, write_result_output_without_started_report, LiveProgressOutput,
};
pub(crate) use shared::{write_stdout_message_lines, write_stdout_record, SharedCheckOutput};
pub(crate) use summary::{summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
// These tests are colocated with the command-output implementation. Dedicated
// tests outside implementation files exercise public CLI behavior instead.
mod tests;
