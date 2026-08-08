use super::AskQueryError;
use crate::app::LazyAppServerRunner;
use crate::check::command::output::{finish_query_output, LiveProgressOutput};
use crate::check::command::{collect_token_usage_for_summary, TokenUsageSummary};
use crate::check::core::QueryResult;

pub(super) fn finish_query_output_and_collect_token_usage_summary(
    started_report: LiveProgressOutput,
    result: &QueryResult,
    human_review_reason: Option<&str>,
    runner: &mut LazyAppServerRunner,
    token_usage_summary: &mut TokenUsageSummary,
) -> Result<(), AskQueryError> {
    // Collect usage even when stdout finishing fails. The outer ask command
    // boundary prints it after this result and all lifecycle cleanup.
    let output_result = finish_query_output(started_report, result, human_review_reason);
    *token_usage_summary = collect_token_usage_for_summary(runner);
    output_result.map_err(AskQueryError::Unreported)
}
