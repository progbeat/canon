use crate::check::core::{CheckRecord, CheckRecordOutcome, CheckResult, SelectedExpectation};
use crate::check::interrogation::state::IN_PLACE_VISIBLE_TREE_OID;
use crate::config_types::AgentConfig;
use crate::hash::full_scope;

// This module enforces `canon check --in-place` mode compatibility. It is not
// the Cached Result implementation and does not define which fields are valid
// in check.yml for ordinary Git-backed checks.
const IN_PLACE_SPEC_PROHIBITED_FIELDS: &[InPlaceProhibitedField] = &[
    InPlaceProhibitedField::DiffFrom,
    InPlaceProhibitedField::Target,
    InPlaceProhibitedField::CachedResultCooldown,
    InPlaceProhibitedField::Ignore,
];

#[derive(Clone, Copy)]
enum InPlaceProhibitedField {
    DiffFrom,
    Target,
    // Cached Result owns normal `cooldown` behavior for Git-backed runs. This
    // variant is only the `canon check --in-place` mode-compatibility rule:
    // in-place config rejects cooldown-cache behavior because that mode treats
    // persisted xpec history as absent.
    CachedResultCooldown,
    Ignore,
}

impl InPlaceProhibitedField {
    fn name(self) -> &'static str {
        match self {
            InPlaceProhibitedField::DiffFrom => "diff-from",
            InPlaceProhibitedField::Target => "target",
            InPlaceProhibitedField::CachedResultCooldown => "cooldown",
            InPlaceProhibitedField::Ignore => "ignore",
        }
    }

    fn is_configured_for(
        self,
        config_agent: &AgentConfig,
        expectation: &SelectedExpectation,
    ) -> bool {
        match self {
            InPlaceProhibitedField::DiffFrom => expectation.diff_from_configured,
            InPlaceProhibitedField::Target => has_explicit_target(expectation),
            InPlaceProhibitedField::CachedResultCooldown => expectation.cooldown.is_some(),
            InPlaceProhibitedField::Ignore => {
                !config_agent.ignore.is_empty() || !expectation.agent.ignore.is_empty()
            }
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

pub(super) fn validate_in_place_global_config(config_agent: &AgentConfig) -> Result<(), String> {
    if config_agent.ignore.is_empty() {
        return Ok(());
    }
    Err("configured ignore invalid in in-place mode".to_string())
}

pub(super) fn invalid_in_place_expectation_records(
    config_agent: &AgentConfig,
    expectations: &[SelectedExpectation],
) -> Result<Vec<CheckRecord>, String> {
    // `expectations` is the selected set for in-place mode. That mode has no
    // cached-result selection step, so these selected expectations go directly
    // to evaluator work unless incompatible fields are reported first.
    let mut records = Vec::new();
    for expectation in expectations {
        let unsupported = in_place_unsupported_fields(config_agent, expectation);
        if !unsupported.is_empty() {
            records.push(in_place_unsupported_expectation_record(
                expectation,
                &unsupported,
            )?);
        }
    }
    Ok(records)
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

fn in_place_unsupported_fields(
    config_agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Vec<&'static str> {
    // These are parsed, valid check.yml fields. This table is only the
    // mode-compatibility list required by the in-place spec: in-place has no Git
    // tree, cached-result lookup, or path hiding, so these otherwise valid
    // features are invalid for expectations selected by that mode.
    IN_PLACE_SPEC_PROHIBITED_FIELDS
        .iter()
        .copied()
        .filter(|field| field.is_configured_for(config_agent, expectation))
        .map(InPlaceProhibitedField::name)
        .collect()
}
