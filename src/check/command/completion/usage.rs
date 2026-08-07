use crate::app::LazyAppServerRunner;
use crate::check::command::output::render_token_usage_summary;
use crate::output::write_stderr_line;
use crate::token_usage::TokenUsage;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

pub(crate) struct TokenUsageSummary {
    rendered_usage: TokenUsage,
}

impl TokenUsageSummary {
    pub(crate) fn unavailable() -> TokenUsageSummary {
        TokenUsageSummary {
            // [kK] The public summary contract represents unavailable usage
            // with zero in every numeric field.
            rendered_usage: TokenUsage::default(),
        }
    }

    fn collected(usage: TokenUsage) -> TokenUsageSummary {
        TokenUsageSummary {
            rendered_usage: usage,
        }
    }

    fn rendered_usage(self) -> TokenUsage {
        self.rendered_usage
    }
}

pub(crate) fn collect_token_usage_for_summary(
    runner: &mut LazyAppServerRunner,
) -> TokenUsageSummary {
    // The summary component owns usage availability and its public rendering.
    // Runtime logs retain their independent per-turn availability.
    if runner.drain_token_usage_updates().is_err() {
        return TokenUsageSummary::unavailable();
    }
    match runner.token_usage() {
        Some(usage) => TokenUsageSummary::collected(usage),
        None => TokenUsageSummary::unavailable(),
    }
}

pub(crate) fn run_with_token_usage_panic_capture<T>(
    runner: &mut LazyAppServerRunner,
    panic_token_usage: &mut TokenUsageSummary,
    run: impl FnOnce(&mut LazyAppServerRunner) -> T,
) -> T {
    let caught_result = catch_unwind(AssertUnwindSafe(|| run(runner)));
    match caught_result {
        Ok(result) => result,
        Err(payload) => {
            // [kK,l] The public command boundary owns the usage line, while the
            // runner is local to the prepared evaluation path. Snapshot usage
            // before unwinding drops it; a secondary collection panic must not
            // replace the original evaluation panic.
            if let Ok(collected) =
                catch_unwind(AssertUnwindSafe(|| collect_token_usage_for_summary(runner)))
            {
                *panic_token_usage = collected;
            }
            resume_unwind(payload)
        }
    }
}

pub(crate) fn print_token_usage_summary(
    token_usage_summary: TokenUsageSummary,
) -> Result<(), String> {
    // This stderr line is part of the public check-output contract. Runtime
    // logs keep the raw per-turn usage records instead of a duplicate aggregate.
    write_stderr_line(&render_token_usage_summary(
        token_usage_summary.rendered_usage(),
    ))
}
