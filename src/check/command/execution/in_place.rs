use crate::check::core::{CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation};
use crate::check::interrogation::state::IN_PLACE_VISIBLE_TREE_OID;
use crate::config_types::DEFAULT_DIFF_FROM;
use crate::hash::full_scope;

pub(super) fn validate_in_place_query_expectation(
    expectation: &SelectedExpectation,
) -> Result<(), String> {
    let invalid = invalid_in_place_fields(expectation);
    if invalid.is_empty() {
        return Ok(());
    }
    let record = invalid_in_place_expectation_record(expectation, &invalid)?;
    Err(format!(
        "{}. ERROR\n{}\nError: {}\nEvidence: {}",
        record.display_id,
        record.question_text(),
        record
            .human_review_reason()
            .expect("invalid in-place records include error"),
        record.evidence
    ))
}

pub(super) fn invalid_in_place_expectation_records(
    expectations: &[SelectedExpectation],
) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    for expectation in expectations {
        let invalid = invalid_in_place_fields(expectation);
        if !invalid.is_empty() {
            records.push(invalid_in_place_expectation_record(expectation, &invalid)?);
        }
    }
    Ok(records)
}

fn invalid_in_place_expectation_record(
    expectation: &SelectedExpectation,
    invalid: &[&'static str],
) -> Result<CheckRecord, String> {
    CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            // CheckRecord represents public ERROR blocks as a failed
            // result with `error: Some(...)`; the output renderer keys
            // off the error field via `requires_human_review`.
            result: CheckResult::Fail,
            observed: "invalid-in-place-expectation".to_string(),
            error: Some("invalid-in-place-expectation".to_string()),
            evidence: format!("selected expectation configures {}", invalid.join(", ")),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: IN_PLACE_VISIBLE_TREE_OID.to_string(),
        },
    )
}

fn invalid_in_place_fields(expectation: &SelectedExpectation) -> Vec<&'static str> {
    let mut invalid = Vec::new();
    if expectation.diff_from != DEFAULT_DIFF_FROM {
        invalid.push("diff-from");
    }
    if expectation.target.is_some() {
        invalid.push("target");
    }
    if expectation.cooldown.is_some() {
        invalid.push("cooldown");
    }
    if !expectation.agent.ignore.is_empty() {
        invalid.push("ignore");
    }
    invalid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::Cooldown;
    use crate::config_types::AgentConfig;

    #[test]
    fn rejects_in_place_expectation_that_needs_git_or_path_hiding() {
        let mut expectation = selected_expectation();
        expectation.diff_from = "HEAD".to_string();
        expectation.cooldown = Some(Cooldown {
            pass_seconds: Some(60),
            fail_seconds: None,
        });
        expectation.agent.ignore = vec!["target/**".to_string()];

        let records = invalid_in_place_expectation_records(&[expectation]).unwrap();
        let [record] = records.as_slice() else {
            panic!("expected one invalid record");
        };

        assert_eq!(record.display_id, "A");
        assert_eq!(record.question_text(), "Can this pass?");
        assert_eq!(
            record.human_review_reason(),
            Some("invalid-in-place-expectation")
        );
        assert_eq!(
            record.evidence,
            "selected expectation configures diff-from, cooldown, ignore"
        );
        assert_eq!(record.scope, full_scope());
        assert_eq!(record.visible_tree_oid, IN_PLACE_VISIBLE_TREE_OID);
    }

    #[test]
    fn collects_every_invalid_in_place_expectation() {
        let mut first = selected_expectation();
        first.diff_from = "HEAD".to_string();
        let mut second = selected_expectation();
        second.id = "bbbbbbbbbbbbbbbbbbbb".to_string();
        second.display_id = "B".to_string();
        second.question = "Can that pass?".to_string();
        second.cooldown = Some(Cooldown {
            pass_seconds: Some(60),
            fail_seconds: None,
        });

        let records = invalid_in_place_expectation_records(&[first, second]).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].display_id, "A");
        assert_eq!(records[1].display_id, "B");
    }

    fn selected_expectation() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "0123456789abcdefghij".to_string(),
            display_id: "A".to_string(),
            question: "Can this pass?".to_string(),
            expected_answer: "yes".to_string(),
            instructions: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }
}
