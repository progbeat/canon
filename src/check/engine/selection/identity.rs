use crate::check::core::ResolvedExpectation;
use crate::config_types::CheckConfig;
use crate::hash::expectation_id;
use std::collections::BTreeSet;
#[cfg(test)]
use std::ffi::OsString;

mod selector;

pub(crate) use selector::select_expectations_with_identities;

// This module owns CLI expectation selector identity matching only.
// Interrogation policy starts after resolved expectations enter check execution
// and the interrogation/session modules.
#[derive(Debug, Clone)]
pub(crate) struct ExpectationIdentity {
    pub(crate) id: String,
    pub(crate) display_id: String,
}

pub(crate) fn selected_expectation_at(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    index: usize,
) -> Result<ResolvedExpectation, String> {
    let identity = identities
        .get(index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    let expectation = config
        .expectations
        .get(index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    Ok(ResolvedExpectation::from_configured(
        identity.id.clone(),
        identity.display_id.clone(),
        expectation,
    ))
}

pub(crate) fn expectation_identities(
    config: &CheckConfig,
) -> Result<Vec<ExpectationIdentity>, String> {
    let ids = config
        .expectations
        .iter()
        .map(|expectation| {
            let rendered_question = &expectation.q;
            let expected_answer = &expectation.a;
            let resolved_instructions = &expectation.question_context;
            expectation_id(
                rendered_question,
                expectation.to.as_str(),
                expected_answer,
                resolved_instructions,
            )
        })
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    for id in &ids {
        if !seen.insert(id.clone()) {
            return Err(format!("duplicate expectation ID: {}", id));
        }
    }
    ids.iter()
        .map(|id| {
            let display_id = minimal_unique_expectation_prefix(id, &ids)
                .ok_or_else(|| format!("expectation ID is not unique: {}", id))?;
            Ok(ExpectationIdentity {
                id: id.clone(),
                display_id,
            })
        })
        .collect()
}

pub(crate) fn minimal_unique_expectation_prefix(id: &str, ids: &[String]) -> Option<String> {
    (1..=id.len()).find_map(|end| {
        let prefix = &id[..end];
        let matches = ids
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .count();
        (matches == 1).then(|| prefix.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{AgentConfig, CheckConfig, Expectation};

    #[test] // xpec: sw
    fn include_selector_selects_matching_unique_id_prefix() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selector = OsString::from(identities[0].display_id.clone());

        let selected =
            select_expectations_with_identities(&config, &identities, &[selector]).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].configured_id(), Some(identities[0].id.as_str()));
    }

    #[test] // xpec: sw
    fn include_selector_accepts_full_id() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selector = OsString::from(identities[1].id.clone());

        let selected =
            select_expectations_with_identities(&config, &identities, &[selector]).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].configured_id(), Some(identities[1].id.as_str()));
    }

    #[test] // xpec: w,sw
    fn include_selector_selects_every_matching_id_prefix() {
        let config = two_expectation_config();
        let identities = vec![
            ExpectationIdentity {
                id: "sharedalpha".to_string(),
                display_id: "shareda".to_string(),
            },
            ExpectationIdentity {
                id: "sharedbeta".to_string(),
                display_id: "sharedb".to_string(),
            },
        ];

        let selected =
            select_expectations_with_identities(&config, &identities, &[OsString::from("shared")])
                .unwrap();

        assert_eq!(configured_ids(&selected), ["sharedalpha", "sharedbeta"]);
    }

    #[test] // xpec: w,sw
    fn overlapping_include_selectors_do_not_duplicate_expectations() {
        let config = two_expectation_config();
        let identities = vec![
            ExpectationIdentity {
                id: "sharedalpha".to_string(),
                display_id: "shareda".to_string(),
            },
            ExpectationIdentity {
                id: "sharedbeta".to_string(),
                display_id: "sharedb".to_string(),
            },
        ];
        let selectors = [OsString::from("shared"), OsString::from("shareda")];

        let selected =
            select_expectations_with_identities(&config, &identities, &selectors).unwrap();

        assert_eq!(configured_ids(&selected), ["sharedalpha", "sharedbeta"]);
    }

    #[test] // xpec: sw
    fn exclusion_selector_selects_all_except_matching_prefix() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selector = OsString::from(format!("not:{}", identities[0].display_id));

        let selected =
            select_expectations_with_identities(&config, &identities, &[selector]).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].configured_id(), Some(identities[1].id.as_str()));
    }

    #[test] // xpec: nK,sw,t
    fn conflicting_include_and_exclusion_is_rejected() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selectors = [
            OsString::from(identities[0].display_id.clone()),
            OsString::from(identities[1].display_id.clone()),
            OsString::from(format!("not:{}", identities[0].display_id)),
        ];

        let error =
            select_expectations_with_identities(&config, &identities, &selectors).unwrap_err();

        assert_eq!(error, "expectation selection could not be completed");
    }

    #[test] // xpec: sw
    fn empty_exclusion_selector_is_rejected() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();

        let err =
            select_expectations_with_identities(&config, &identities, &[OsString::from("not:")])
                .unwrap_err();

        assert_eq!(err, "expectation selector must not be empty");
    }

    #[test] // xpec: 6,sw,t
    fn repeated_and_unknown_exclusions_have_the_same_observable_selection() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let short_selector = OsString::from(format!("not:{}", identities[0].display_id));
        let full_selector = OsString::from(format!("not:{}", identities[0].id));
        let unknown_selector = OsString::from("not:00000000000000000000");

        let repeated = select_expectations_with_identities(
            &config,
            &identities,
            &[short_selector, full_selector.clone()],
        )
        .unwrap();
        let unknown = select_expectations_with_identities(
            &config,
            &identities,
            &[unknown_selector, full_selector],
        )
        .unwrap();

        assert_eq!(configured_ids(&repeated), configured_ids(&unknown));
    }

    #[test] // xpec: 6,sw,t
    fn exclusion_removes_hidden_identity_from_include_prefix_matching() {
        let config = two_expectation_config();
        let identities = vec![
            ExpectationIdentity {
                id: "sharedalpha".to_string(),
                display_id: "shareda".to_string(),
            },
            ExpectationIdentity {
                id: "sharedbeta".to_string(),
                display_id: "sharedb".to_string(),
            },
        ];
        let selectors = [OsString::from("shared"), OsString::from("not:sharedalpha")];

        let selected =
            select_expectations_with_identities(&config, &identities, &selectors).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].configured_id(), Some("sharedbeta"));
    }

    #[test] // xpec: 6,nK,t
    fn excluded_include_and_unknown_include_have_the_same_error() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let included = OsString::from(identities[0].id.clone());
        let excluded = OsString::from(format!("not:{}", identities[0].id));
        let other = OsString::from(identities[1].id.clone());
        let unknown = OsString::from("00000000000000000000");

        let excluded_error = select_expectations_with_identities(
            &config,
            &identities,
            &[included.clone(), excluded.clone()],
        )
        .unwrap_err();
        let unknown_error = select_expectations_with_identities(
            &config,
            &identities,
            std::slice::from_ref(&unknown),
        )
        .unwrap_err();
        let known_candidate_with_other_error = select_expectations_with_identities(
            &config,
            &identities,
            &[included, other.clone(), excluded.clone()],
        )
        .unwrap_err();
        let unknown_candidate_with_other_error =
            select_expectations_with_identities(&config, &identities, &[unknown, other, excluded])
                .unwrap_err();

        assert_eq!(excluded_error, unknown_error);
        assert_eq!(
            unknown_error,
            "expectation selection could not be completed"
        );
        assert_eq!(
            known_candidate_with_other_error,
            unknown_candidate_with_other_error
        );
    }

    fn two_expectation_config() -> CheckConfig {
        CheckConfig {
            version: 1,
            agent: AgentConfig::implementation_default(),
            expectations: vec![
                expectation("Does alpha pass?"),
                expectation("Does beta pass?"),
            ],
        }
    }

    fn expectation(question: &str) -> Expectation {
        Expectation {
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            q: question.to_string(),
            a: "yes".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::implementation_default(),
            cooldown: None,
            q_scope: Default::default(),
            in_place_compatibility: Default::default(),
        }
    }

    fn configured_ids(expectations: &[ResolvedExpectation]) -> Vec<&str> {
        expectations
            .iter()
            .filter_map(ResolvedExpectation::configured_id)
            .collect()
    }
}
