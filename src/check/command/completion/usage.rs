use crate::app::LazyAppServerRunner;
use crate::check::command::output::render_token_usage_summary;
use crate::output::write_stderr_line;
use crate::token_usage_types::TokenUsage;

pub(crate) fn collect_check_token_usage(
    runner: &mut LazyAppServerRunner,
) -> Result<TokenUsage, String> {
    runner
        .drain_token_usage_updates()
        .map_err(|err| err.to_string())?;
    Ok(runner.token_usage().unwrap_or_default())
}

pub(crate) fn print_token_usage_summary(usage: Option<TokenUsage>) -> Result<(), String> {
    // This stderr line is part of the public check-output contract. Runtime
    // logs keep the raw per-turn usage records instead of a duplicate aggregate.
    let usage = usage.unwrap_or_default();
    // Keep the model-agnostic reference-cost metric on the same code path as
    // raw usage reporting without changing the documented public line.
    let _reference_token_cost = usage.reference_token_cost();
    write_stderr_line(&render_token_usage_summary(usage))
}
