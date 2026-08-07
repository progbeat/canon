use super::super::trailer::attempt_independent_finally_effects;
use super::finish::combine_failure_effect_results;
use crate::check::command::output::{
    command_error_feedback_messages, continue_evaluation_message, write_stdout_message_lines,
    write_summary_line, CheckFeedbackContext,
};
use crate::check::command::{
    check_feedback_messages, print_token_usage_summary, TokenUsageSummary,
};
use crate::check::core::CheckRunReport;
use crate::cli::CommandError;
use std::io;
use std::time::Instant;

#[derive(Clone, Copy)]
pub(in crate::check::command::workflow) struct CheckFailureOutput {
    started: Instant,
    collection: CheckFailureCollection,
    default_feedback_eligible: bool,
    feedback_context: Option<CheckFeedbackContext>,
    lifecycle_started: bool,
}

#[derive(Clone, Copy)]
enum CheckFailureCollection {
    CollectionNotAttempted,
    CollectionFailed,
    Collected { pending: usize },
    ReadyForEvaluation { pending: usize },
}

impl CheckFailureOutput {
    pub(in crate::check::command::workflow) fn needs_pending_collection(self) -> bool {
        self.default_feedback_eligible
            && matches!(
                self.collection,
                CheckFailureCollection::CollectionNotAttempted
            )
    }

    pub(in crate::check::command::workflow) fn mark_collection_failed(&mut self) {
        if matches!(
            self.collection,
            CheckFailureCollection::CollectionNotAttempted
        ) {
            self.collection = CheckFailureCollection::CollectionFailed;
        }
    }

    pub(in crate::check::command::workflow) fn mark_collection_complete(&mut self, pending: usize) {
        self.collection = match self.collection {
            CheckFailureCollection::ReadyForEvaluation { .. } => {
                CheckFailureCollection::ReadyForEvaluation { pending }
            }
            CheckFailureCollection::CollectionNotAttempted
            | CheckFailureCollection::CollectionFailed
            | CheckFailureCollection::Collected { .. } => {
                CheckFailureCollection::Collected { pending }
            }
        };
    }

    pub(in crate::check::command::workflow) fn mark_ready_for_evaluation(&mut self) {
        let pending = match self.collection {
            CheckFailureCollection::Collected { pending } => pending,
            CheckFailureCollection::ReadyForEvaluation { .. } => return,
            CheckFailureCollection::CollectionNotAttempted
            | CheckFailureCollection::CollectionFailed => {
                panic!("evaluation readiness requires completed xpec collection")
            }
        };
        self.collection = CheckFailureCollection::ReadyForEvaluation { pending };
    }

    pub(in crate::check::command::workflow) fn lifecycle_started(self) -> bool {
        self.lifecycle_started
    }

    pub(in crate::check::command::workflow) fn mark_lifecycle_started(&mut self) {
        self.lifecycle_started = true;
    }

    pub(in crate::check::command::workflow) fn with_feedback_context(
        mut self,
        feedback_context: CheckFeedbackContext,
    ) -> Self {
        self.feedback_context = Some(feedback_context);
        self
    }

    fn pending_count(self) -> usize {
        // [kK] Before config collection, or when collection itself fails, there
        // are zero collected xpecs. After collection, every xpec without a
        // result is pending. Keep those states and the later
        // evaluation-readiness transition explicit instead of fabricating
        // result records.
        match self.collection {
            CheckFailureCollection::CollectionNotAttempted
            | CheckFailureCollection::CollectionFailed => 0,
            CheckFailureCollection::Collected { pending }
            | CheckFailureCollection::ReadyForEvaluation { pending } => pending,
        }
    }

    pub(in crate::check::command::workflow) fn feedback_messages_for_report(
        self,
        report: &CheckRunReport,
    ) -> Vec<String> {
        if !self.default_feedback_eligible {
            return Vec::new();
        }
        let feedback_context = self
            .feedback_context
            .expect("default feedback eligibility establishes its assertion context");
        match self.collection {
            // Collection has not been attempted, so evaluation is necessarily
            // still pending even though no complete xpec outcome domain exists
            // for the summary. Use the canonical pending feedback text
            // directly; do not invent an xpec count.
            CheckFailureCollection::CollectionNotAttempted => {
                feedback_context.assert_default_against_head();
                vec![continue_evaluation_message()]
            }
            // [kK] A collection error is the reported command error; rerunning
            // cannot continue evaluation until the user fixes that error.
            CheckFailureCollection::CollectionFailed | CheckFailureCollection::Collected { .. } => {
                command_error_feedback_messages(feedback_context)
            }
            CheckFailureCollection::ReadyForEvaluation { .. } => {
                check_feedback_messages(report, feedback_context, None)
            }
        }
    }
}

pub(in crate::check::command::workflow) fn requested_check_output(
    started: Instant,
    default_feedback_eligible: bool,
) -> CheckFailureOutput {
    CheckFailureOutput {
        started,
        collection: CheckFailureCollection::CollectionNotAttempted,
        default_feedback_eligible,
        feedback_context: default_feedback_eligible
            .then(CheckFeedbackContext::for_default_against_head),
        lifecycle_started: false,
    }
}

pub(in crate::check::command::workflow) fn write_unconditional_check_trailer_and_feedback(
    output: CheckFailureOutput,
) -> Result<(), CommandError> {
    // [kK] The check contract makes these three `finally` effects unconditional
    // and independent of the command-specific diagnostic already flushed at
    // the failure boundary. Command examples such as [D8] specify those
    // diagnostic and feedback lines without overriding this shared trailer.
    // Compute every result before combining errors so a failure in any output
    // channel cannot suppress token usage, summary, or feedback.
    let pending = output.pending_count();
    let report = CheckRunReport {
        records: Vec::new(),
        cached_passes: Vec::new(),
        pending,
    };
    write_unconditional_check_trailer_and_feedback_for_report(
        output,
        &report,
        TokenUsageSummary::unavailable(),
    )
}

pub(in crate::check::command::workflow) fn write_unconditional_check_trailer_and_feedback_for_report(
    output: CheckFailureOutput,
    report: &CheckRunReport,
    token_usage_summary: TokenUsageSummary,
) -> Result<(), CommandError> {
    // [2Z,kK] A panic after evaluation has started uses the caller-owned
    // progress report, so completed and cached outcomes are not relabeled as
    // pending by the unconditional trailer and feedback path.
    let [token_usage_result, summary_result, feedback_result] =
        attempt_independent_finally_effects([
            Box::new(move || {
                print_token_usage_summary(token_usage_summary).map_err(CommandError::from)
            }),
            Box::new(|| {
                write_summary_line(&mut io::stdout(), report, output.started.elapsed())
                    .map_err(CommandError::from)
            }),
            Box::new(|| write_check_failure_feedback(output, report)),
        ]);
    combine_failure_effect_results([token_usage_result, summary_result, feedback_result])
}

pub(in crate::check::command::workflow) fn write_check_failure_feedback(
    output: CheckFailureOutput,
    report: &CheckRunReport,
) -> Result<(), CommandError> {
    // [2Z,kK] Feedback is an independent `finally` effect for a selected
    // default-source run. Collection that has not been attempted uses the
    // canonical continuation action without inventing xpec counts; collection
    // failure or a collected config that fails before evaluation readiness
    // uses the reported-error action. Once evaluation is ready, failed or
    // pending report actions remain authoritative. Attempt feedback after both
    // trailer writes even if either output channel failed.
    write_stdout_message_lines(
        &mut io::stdout(),
        output.feedback_messages_for_report(report),
        "pre-report check feedback",
    )
    .map_err(CommandError::from)
}
