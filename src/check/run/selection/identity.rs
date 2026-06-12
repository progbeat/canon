use super::cooldown::parse_cooldown;
use crate::check::core::SelectedExpectation;
use crate::config_types::CheckConfig;
use crate::hash::expectation_id;
use std::collections::BTreeSet;
use std::ffi::OsString;

const EXCLUSION_SELECTOR_PREFIX: &str = "not:";

#[derive(Debug, Clone)]
pub(crate) struct ExpectationIdentity {
    pub(crate) id: String,
    pub(crate) display_id: String,
}

pub(crate) fn select_expectations_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    let mut selected_indexes = Vec::new();
    if args.is_empty() {
        selected_indexes.extend(0..config.expectations.len());
    } else {
        let selectors = parse_expectation_selectors(args)?;
        let mut excluded_indexes = BTreeSet::new();
        let has_include = selectors
            .iter()
            .any(|selector| matches!(selector, ExpectationSelector::Include(_)));
        if !has_include {
            selected_indexes.extend(0..config.expectations.len());
        }
        let mut seen_included = BTreeSet::new();
        for selector in selectors {
            match selector {
                ExpectationSelector::Include(text) => {
                    let matches = matching_expectation_indexes(identities, &text);
                    let index = match matches.as_slice() {
                        [] => return Err(format!("unknown expectation selector: {}", text)),
                        [index] => *index,
                        _ => return Err(format!("ambiguous expectation selector: {}", text)),
                    };
                    if !seen_included.insert(index) {
                        return Err(format!("duplicate expectation selector: {}", text));
                    }
                    selected_indexes.push(index);
                }
                ExpectationSelector::Exclude(text) => {
                    let matches = matching_expectation_indexes(identities, &text);
                    if matches.is_empty() {
                        return Err(format!(
                            "unknown expectation selector: {}{}",
                            EXCLUSION_SELECTOR_PREFIX, text
                        ));
                    }
                    excluded_indexes.extend(matches);
                }
            }
        }
        selected_indexes.retain(|index| !excluded_indexes.contains(index));
    }

    selected_indexes
        .into_iter()
        .map(|index| selected_expectation_at(config, identities, index, true))
        .collect::<Result<Vec<_>, _>>()
}

enum ExpectationSelector {
    Include(String),
    Exclude(String),
}

fn parse_expectation_selectors(args: &[OsString]) -> Result<Vec<ExpectationSelector>, String> {
    let mut selectors = Vec::new();
    let mut seen = BTreeSet::new();
    for arg in args {
        let text = arg
            .to_str()
            .ok_or("expectation selector must be valid UTF-8".to_string())?;
        if text.is_empty() {
            return Err("expectation selector must not be empty".to_string());
        }
        if !seen.insert(text.to_string()) {
            return Err(format!("duplicate expectation selector: {}", text));
        }
        if let Some(excluded) = text.strip_prefix(EXCLUSION_SELECTOR_PREFIX) {
            if excluded.is_empty() {
                return Err("expectation selector must not be empty".to_string());
            }
            selectors.push(ExpectationSelector::Exclude(excluded.to_string()));
        } else {
            selectors.push(ExpectationSelector::Include(text.to_string()));
        }
    }
    Ok(selectors)
}

pub(crate) fn selected_expectation_at(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    index: usize,
    include_cooldown: bool,
) -> Result<SelectedExpectation, String> {
    let identity = identities
        .get(index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    let expectation = config
        .expectations
        .get(index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    let cooldown = if include_cooldown {
        expectation
            .cooldown
            .as_ref()
            .map(parse_cooldown)
            .transpose()?
    } else {
        None
    };
    Ok(SelectedExpectation {
        number: index + 1,
        id: identity.id.clone(),
        display_id: identity.display_id.clone(),
        question: expectation.q.clone(),
        expected_answer: expectation.a.clone(),
        instructions: expectation.instructions.clone(),
        target: expectation.target.clone(),
        question_answer_only: expectation.question_answer_only,
        agent: expectation.agent.clone(),
        cooldown,
    })
}

pub(crate) fn expectation_identities(
    config: &CheckConfig,
) -> Result<Vec<ExpectationIdentity>, String> {
    let ids = config
        .expectations
        .iter()
        .map(|expectation| {
            let rendered_question = &expectation.q;
            let resolved_instructions = &expectation.instructions;
            expectation_id(rendered_question, resolved_instructions)
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

fn matching_expectation_indexes(identities: &[ExpectationIdentity], selector: &str) -> Vec<usize> {
    identities
        .iter()
        .enumerate()
        .filter_map(|(index, identity)| identity.id.starts_with(selector).then_some(index))
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

    #[test]
    fn exclusion_selector_selects_all_except_matching_prefix() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selector = OsString::from(format!("not:{}", identities[0].display_id));

        let selected =
            select_expectations_with_identities(&config, &identities, &[selector]).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, identities[1].id);
    }

    #[test]
    fn exclusion_selector_filters_explicit_includes() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let selectors = [
            OsString::from(identities[0].display_id.clone()),
            OsString::from(identities[1].display_id.clone()),
            OsString::from(format!("not:{}", identities[0].display_id)),
        ];

        let selected =
            select_expectations_with_identities(&config, &identities, &selectors).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, identities[1].id);
    }

    #[test]
    fn empty_exclusion_selector_is_rejected() {
        let config = two_expectation_config();
        let identities = expectation_identities(&config).unwrap();

        let err =
            select_expectations_with_identities(&config, &identities, &[OsString::from("not:")])
                .unwrap_err();

        assert_eq!(err, "expectation selector must not be empty");
    }

    fn two_expectation_config() -> CheckConfig {
        CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: AgentConfig::implementation_default(),
            expectations: vec![
                expectation("Does alpha pass?"),
                expectation("Does beta pass?"),
            ],
        }
    }

    fn expectation(question: &str) -> Expectation {
        Expectation {
            q: question.to_string(),
            a: "yes".to_string(),
            instructions: String::new(),
            target: None,
            question_answer_only: true,
            agent: AgentConfig::implementation_default(),
            cooldown: None,
        }
    }
}
