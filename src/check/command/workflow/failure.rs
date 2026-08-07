mod finish;
mod output;

#[derive(Clone, Copy, Default)]
pub(super) enum CheckPublicOutputProgress {
    #[default]
    None,
    Trailer,
    All,
}

impl CheckPublicOutputProgress {
    pub(super) fn needs_trailer(self) -> bool {
        matches!(self, Self::None)
    }

    pub(super) fn needs_feedback(self) -> bool {
        !matches!(self, Self::All)
    }

    pub(super) fn mark_trailer_attempted(&mut self) {
        if matches!(*self, Self::None) {
            *self = Self::Trailer;
        }
    }

    pub(super) fn mark_feedback_attempted(&mut self) {
        // [kK] Normal completion attempts feedback only after the trailer
        // attempt returned. Failure paths transfer both effects together only
        // after their preceding diagnostic attempt returned.
        match self {
            Self::None => {
                unreachable!("feedback cannot precede the public check trailer")
            }
            Self::Trailer | Self::All => *self = Self::All,
        }
    }

    pub(super) fn mark_all_attempted(&mut self) {
        *self = Self::All;
    }
}

// Public failure-output behavior is covered at the CLI boundary by the
// validation and state-write example tests. Keep lifecycle-state construction
// private here; the exhaustive collection enum in `output` owns its invariants.

pub(super) use finish::{
    combine_failure_effect_results, fail_check_before_lifecycle, fail_check_before_selection,
    finish_check_error_report, or_fail_at_selection_boundary, or_finalize,
    start_check_with_candidates_or_fail, CheckErrorReportFinish, SelectionBoundary,
};
pub(super) use output::{
    requested_check_output, write_check_failure_feedback,
    write_unconditional_check_trailer_and_feedback,
    write_unconditional_check_trailer_and_feedback_for_report, CheckFailureOutput,
};

#[cfg(test)]
mod tests {
    use super::CheckPublicOutputProgress;

    #[test] // xpec: kK
    fn public_output_progress_keeps_feedback_eligible_after_trailer_attempt() {
        let mut progress = CheckPublicOutputProgress::default();

        assert!(progress.needs_trailer());
        assert!(progress.needs_feedback());
        progress.mark_trailer_attempted();
        assert!(!progress.needs_trailer());
        assert!(progress.needs_feedback());
        progress.mark_feedback_attempted();
        assert!(!progress.needs_feedback());
    }
}
