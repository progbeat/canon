use crate::check::core::{
    for_each_unique_report_record, CachedExpectation, CheckRecord, CheckRunReport,
};

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    cached: Vec<CachedExpectation>,
    counts: CheckRunReportCounts,
) -> CheckRunReport {
    CheckRunReport {
        records,
        cached,
        blocked: None,
        skipped: counts.skipped,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) skipped: usize,
}

pub(crate) fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CachedExpectation],
) -> usize {
    let mut unique_records = 0usize;
    for_each_unique_report_record(records, cached, |_| unique_records += 1);
    total_expectations.saturating_sub(unique_records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{
        CachedExpectation, CheckRecord, CheckResult, SelectedExpectation, ERROR_INVALID_QUESTION,
    };
    use crate::config_types::{AgentConfig, DEFAULT_DIFF_FROM};

    #[test] // xpec: T
    fn cached_human_review_counts_as_error_not_pending() {
        let id = "11111111111111111111";
        let cached = vec![CachedExpectation {
            expectation: selected_expectation(id),
            record: human_review_record(id),
        }];

        // xpec: T
        assert_eq!(skipped_count(1, &[], &cached), 0);
    }

    fn selected_expectation(id: &str) -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: id.to_string(),
            display_id: "j".to_string(),
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }

    fn human_review_record(id: &str) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Fail,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: String::new(),
            error: Some(ERROR_INVALID_QUESTION.to_string()),
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            question_scope_suggestion: None,
            visible_tree_oid: "visible".to_string(),
            id: id.to_string(),
            display_id: "j".to_string(),
        }
    }
}
