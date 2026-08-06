use crate::app::LazyAppServerRunner;
use crate::check::command::output::{summary_outcome_counts, write_summary_line};
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::{
    collect_token_usage_for_summary, print_token_usage_summary, TokenUsageSummary,
};
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use std::any::Any;
use std::io::Write;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::time::Instant;

pub(super) struct CompletedCheckRun {
    pub(super) report: CheckRunReport,
    pub(super) error: Option<String>,
    pub(super) failure_history_feedback: Option<crate::xpec_state::FailureHistoryFeedback>,
}

pub(super) fn check_report_passed(report: &CheckRunReport) -> bool {
    let counts = summary_outcome_counts(report);
    counts.failed == 0 && counts.pending == 0
}

pub(super) fn check_command_emits_feedback(command: &CheckCommandArgs) -> bool {
    !command.in_place && command.sources_have_command_default_values
}

pub(super) fn write_check_trailer(
    runner: &mut LazyAppServerRunner,
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
    public_output_progress: &mut CheckPublicOutputProgress,
) -> Result<(), String> {
    let token_usage_summary = collect_token_usage_for_summary(runner);
    // [w] Collection may panic while the outer fallback still owns every
    // public effect. Transfer ownership only when the independently protected
    // token-usage and summary attempts are about to start.
    public_output_progress.mark_trailer_attempted();
    write_check_trailer_with_token_usage_summary(
        result_output,
        report,
        started,
        token_usage_summary,
    )
}

pub(super) fn write_check_trailer_with_token_usage_summary(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    started: Instant,
    token_usage_summary: TokenUsageSummary,
) -> Result<(), String> {
    write_required_trailer_parts(
        || print_token_usage_summary(token_usage_summary),
        || write_summary_line(result_output, report, started.elapsed()),
    )
}

pub(super) fn write_required_trailer_parts(
    write_token_usage: impl FnOnce() -> Result<(), String>,
    write_summary: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // [w] These are independent `finally` outputs. Attempt both after either
    // an ordinary write error or a panic, then resume the first panic so
    // fallback handling cannot replace the original failure.
    let [token_usage_result, summary_result] =
        attempt_independent_finally_effects([Box::new(write_token_usage), Box::new(write_summary)]);
    match (token_usage_result, summary_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(token_error), Err(summary_error)) => Err(format!(
            "{token_error}; also failed to write check summary: {summary_error}"
        )),
    }
}

pub(in crate::check::command::workflow) fn attempt_independent_finally_effects<
    'a,
    E,
    const N: usize,
>(
    effects: [Box<dyn FnOnce() -> Result<(), E> + 'a>; N],
) -> [Result<(), E>; N] {
    let mut first_panic: Option<Box<dyn Any + Send>> = None;
    let mut results = Vec::with_capacity(N);
    for effect in effects {
        match catch_unwind(AssertUnwindSafe(effect)) {
            Ok(result) => results.push(result),
            Err(payload) => {
                if first_panic.is_none() {
                    first_panic = Some(payload);
                }
            }
        }
    }
    if let Some(payload) = first_panic {
        resume_unwind(payload);
    }
    // [w] Without a captured panic, every effect returned exactly one result.
    match results.try_into() {
        Ok(results) => results,
        Err(_) => unreachable!("every non-panicking finally effect must return a result"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::command::args::parse_check_command_args;
    use std::ffi::OsString;

    #[test] // xpec: w
    fn feedback_allows_normalized_default_config_path() {
        let command = command_with_config_path("./.canon/check.yml");

        assert!(check_command_emits_feedback(&command));
    }

    #[test] // xpec: w
    fn feedback_rejects_non_default_config_path() {
        let command = command_with_config_path(".canon/other.yml");

        assert!(!check_command_emits_feedback(&command));
    }

    #[test] // xpec: w
    fn feedback_rejects_in_place_checks() {
        let mut command = command_with_config_path(".canon/check.yml");
        command.in_place = true;

        assert!(!check_command_emits_feedback(&command));
    }

    #[test] // xpec: w
    fn feedback_allows_explicit_selectors_with_default_command_sources() {
        let mut command = command_with_config_path(".canon/check.yml");
        command.options.selectors.push("a7F".into());

        assert!(check_command_emits_feedback(&command));
    }

    // xpec: T5
    #[test]
    fn check_report_passed_rejects_pending_expectations() {
        let report = CheckRunReport {
            records: Vec::new(),
            cached_passes: Vec::new(),
            pending: 1,
        };

        // xpec: T5
        assert!(!check_report_passed(&report));
    }

    #[test] // xpec: w
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

    #[test] // xpec: w
    fn trailer_attempts_summary_after_token_usage_panic() {
        let mut token_usage_attempted = false;
        let mut summary_attempted = false;

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = write_required_trailer_parts(
                || {
                    token_usage_attempted = true;
                    panic!("token usage panicked")
                },
                || {
                    summary_attempted = true;
                    Ok(())
                },
            );
        }))
        .unwrap_err();

        assert!(token_usage_attempted);
        assert!(summary_attempted);
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"token usage panicked"));
    }

    #[test] // xpec: w
    fn finally_effects_attempt_feedback_after_trailer_panic() {
        let mut token_usage_attempted = false;
        let mut summary_attempted = false;
        let mut feedback_attempted = false;

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: [Result<(), String>; 3] = attempt_independent_finally_effects([
                Box::new(|| {
                    token_usage_attempted = true;
                    panic!("token usage panicked")
                }),
                Box::new(|| {
                    summary_attempted = true;
                    Ok(())
                }),
                Box::new(|| {
                    feedback_attempted = true;
                    Ok(())
                }),
            ]);
        }))
        .unwrap_err();

        assert!(token_usage_attempted);
        assert!(summary_attempted);
        assert!(feedback_attempted);
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"token usage panicked"));
    }

    fn command_with_config_path(config_path: &str) -> CheckCommandArgs {
        parse_check_command_args(
            &[OsString::from("--config"), OsString::from(config_path)],
            false,
        )
        .unwrap()
    }
}
