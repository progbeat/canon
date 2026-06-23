use crate::check::core::{CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation};
use crate::check::interrogation::state::IN_PLACE_VISIBLE_TREE_OID;
use crate::check::run::selection::{selected_expectation_at, ExpectationIdentity};
use crate::config_types::CheckConfig;
use crate::config_types::DEFAULT_DIFF_FROM;
use crate::hash::full_scope;

// This module enforces `canon check --in-place` mode compatibility. It is not
// the Cached Result implementation and does not define which fields are valid
// in check.yml for ordinary Git-backed checks.
const IN_PLACE_SPEC_PROHIBITED_FIELDS: &[InPlaceProhibitedField] = &[
    InPlaceProhibitedField::DiffFrom,
    InPlaceProhibitedField::Target,
    InPlaceProhibitedField::Cooldown,
    InPlaceProhibitedField::Ignore,
];

#[derive(Clone, Copy)]
enum InPlaceProhibitedField {
    DiffFrom,
    Target,
    // Cached Result owns the normal `cooldown` behavior; this variant exists
    // only because `canon check --in-place` explicitly prohibits cache inputs.
    Cooldown,
    Ignore,
}

impl InPlaceProhibitedField {
    fn name(self) -> &'static str {
        match self {
            InPlaceProhibitedField::DiffFrom => "diff-from",
            InPlaceProhibitedField::Target => "target",
            InPlaceProhibitedField::Cooldown => "cooldown",
            InPlaceProhibitedField::Ignore => "ignore",
        }
    }

    fn is_configured_for(self, expectation: &SelectedExpectation) -> bool {
        match self {
            InPlaceProhibitedField::DiffFrom => expectation.diff_from != DEFAULT_DIFF_FROM,
            InPlaceProhibitedField::Target => has_explicit_target(expectation),
            InPlaceProhibitedField::Cooldown => expectation.cooldown.is_some(),
            InPlaceProhibitedField::Ignore => !expectation.agent.ignore.is_empty(),
        }
    }
}

fn has_explicit_target(expectation: &SelectedExpectation) -> bool {
    // Omitted target means the default project target and is not a configured
    // in-place feature. Explicit `target: project` and `target: diff` are
    // rejected uniformly by presence only; the target value itself remains
    // prompt-rendering data.
    expectation.target.is_some()
}

pub(super) fn validate_in_place_query_expectation(
    expectation: &SelectedExpectation,
) -> Result<(), String> {
    let unsupported = in_place_unsupported_fields(expectation);
    if unsupported.is_empty() {
        return Ok(());
    }
    let record = in_place_unsupported_expectation_record(expectation, &unsupported)?;
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
        let unsupported = in_place_unsupported_fields(expectation);
        if !unsupported.is_empty() {
            records.push(in_place_unsupported_expectation_record(
                expectation,
                &unsupported,
            )?);
        }
    }
    Ok(records)
}

pub(super) fn invalid_in_place_config_records(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
) -> Result<Vec<CheckRecord>, String> {
    let expectations = (0..config.expectations.len())
        .map(|index| selected_expectation_at(config, identities, index, true))
        .collect::<Result<Vec<_>, _>>()?;
    invalid_in_place_expectation_records(&expectations)
}

fn in_place_unsupported_expectation_record(
    expectation: &SelectedExpectation,
    unsupported: &[&'static str],
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
            evidence: format!(
                "configured {} invalid in in-place mode",
                unsupported.join(", ")
            ),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: IN_PLACE_VISIBLE_TREE_OID.to_string(),
        },
    )
}

fn in_place_unsupported_fields(expectation: &SelectedExpectation) -> Vec<&'static str> {
    // These are parsed, valid check.yml fields. This table is only the
    // mode-compatibility list required by the in-place spec: in-place has no Git
    // tree, cached-result lookup, or path hiding, so these otherwise valid
    // features are invalid only for expectations evaluated by that mode.
    IN_PLACE_SPEC_PROHIBITED_FIELDS
        .iter()
        .copied()
        .filter(|field| field.is_configured_for(expectation))
        .map(InPlaceProhibitedField::name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::Cooldown;
    use crate::check::run::selection::expectation_identities;
    use crate::config_types::{AgentConfig, CheckConfig, Expectation, ExpectationTarget};
    use std::collections::BTreeMap;

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
            "configured diff-from, cooldown, ignore invalid in in-place mode"
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
        second.target = Some(ExpectationTarget::Diff);

        let records = invalid_in_place_expectation_records(&[first, second]).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].display_id, "A");
        assert_eq!(records[1].display_id, "B");
    }

    #[test]
    fn in_place_rejects_explicit_project_and_diff_targets_uniformly() {
        let mut project = selected_expectation();
        project.target = Some(ExpectationTarget::Project);
        let mut diff = selected_expectation();
        diff.id = "bbbbbbbbbbbbbbbbbbbb".to_string();
        diff.display_id = "B".to_string();
        diff.question = "Can that pass?".to_string();
        diff.target = Some(ExpectationTarget::Diff);

        let records = invalid_in_place_expectation_records(&[project, diff]).unwrap();

        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|record| record.evidence == "configured target invalid in in-place mode"));
    }

    #[test]
    fn config_records_include_unselected_invalid_expectations() {
        let mut config = CheckConfig {
            version: 1,
            presets: BTreeMap::new(),
            agent: AgentConfig::default(),
            expectations: vec![
                config_expectation("Can this pass?"),
                config_expectation("Can that pass?"),
            ],
        };
        config.expectations[1].cooldown = Some(crate::config_types::CooldownConfig::Compact(
            "1d".to_string(),
        ));
        config.expectations[1].target = Some(ExpectationTarget::Diff);
        let identities = expectation_identities(&config).unwrap();
        let selected =
            selected_expectation_at(&config, &identities, 0, true).expect("valid selected");

        assert!(invalid_in_place_expectation_records(&[selected])
            .unwrap()
            .is_empty());
        let records = invalid_in_place_config_records(&config, &identities).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].display_id, identities[1].display_id);
        assert_eq!(records[0].question_text(), "Can that pass?");
        assert_eq!(
            records[0].evidence,
            "configured target, cooldown invalid in in-place mode"
        );
    }

    #[test]
    fn in_place_config_records_reject_cooldown_without_making_it_invalid_config() {
        let mut config = CheckConfig {
            version: 1,
            presets: BTreeMap::new(),
            agent: AgentConfig::default(),
            expectations: vec![config_expectation("Can this pass?")],
        };
        config.expectations[0].cooldown = Some(crate::config_types::CooldownConfig::Compact(
            "1d".to_string(),
        ));
        let identities = expectation_identities(&config).unwrap();

        let records = invalid_in_place_config_records(&config, &identities).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].evidence,
            "configured cooldown invalid in in-place mode"
        );
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

    fn config_expectation(question: &str) -> Expectation {
        Expectation {
            q: question.to_string(),
            a: "yes".to_string(),
            instructions: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }
}
