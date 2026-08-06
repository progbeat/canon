use crate::check::{
    CheckRecord, CheckRecordOutcome, CheckResult, ParsedAnswer, ResolvedExpectation,
};

pub(crate) fn record_from_response(
    expectation: &ResolvedExpectation,
    response: ParsedAnswer,
    visible_tree_oid: Option<String>,
    diff_from: Option<String>,
    diff_from_tree_oid: Option<String>,
    diff_from_tree_oid_abbrev: Option<String>,
) -> Result<CheckRecord, String> {
    let expected_answer = expectation.expected_answer();
    let ParsedAnswer {
        observed,
        error,
        evidence,
        scope,
        q_scope_suggestion,
    } = response;
    let result = CheckResult::from_evaluation(expected_answer, &observed, error.as_deref());
    let outcome = CheckRecordOutcome::new(result, observed, error, evidence, scope);
    CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            q_scope_suggestion,
            visible_tree_oid,
            diff_from,
            diff_from_tree_oid,
            diff_from_tree_oid_abbrev,
            ..outcome
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::EvaluationAnswer;
    use crate::config_types::{AgentConfig, ExpectationTo, DEFAULT_DIFF_FROM};

    fn expectation() -> ResolvedExpectation {
        ResolvedExpectation {
            kind: crate::check::ResolvedExpectationKind::Configured {
                id: "id".to_string(),
            },
            display_id: "q".to_string(),
            to: ExpectationTo::Agent,
            rank: 0,
            question: "question".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::default(),
            cooldown: None,
            q_scope: Default::default(),
        }
    }

    #[test] // xpec: 4k
    fn outcome_is_independent_of_evidence_content() {
        let result_for = |evidence: &str| {
            record_from_response(
                &expectation(),
                ParsedAnswer::answer(
                    EvaluationAnswer::new("yes".to_string()),
                    evidence.to_string(),
                    None,
                ),
                None,
                None,
                None,
                None,
            )
            .expect("valid response")
            .result
        };

        assert_eq!(result_for("supports yes"), result_for("claims no")); // xpec: 4k
        assert_eq!(result_for("arbitrary evidence"), CheckResult::Pass); // xpec: 4k
    }
}
