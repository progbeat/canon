use super::cooldown::parse_cooldown;
use crate::check::core::SelectedExpectation;
use crate::config_types::CheckConfig;
use crate::hash::expectation_id;
use std::collections::BTreeSet;
use std::ffi::OsString;

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
        let mut seen = BTreeSet::new();
        for arg in args {
            let text = arg
                .to_str()
                .ok_or("expectation selector must be valid UTF-8".to_string())?;
            if text.is_empty() {
                return Err("expectation selector must not be empty".to_string());
            }
            let matches = matching_expectation_indexes(identities, text);
            let index = match matches.as_slice() {
                [] => return Err(format!("unknown expectation selector: {}", text)),
                [index] => *index,
                _ => return Err(format!("ambiguous expectation selector: {}", text)),
            };
            if !seen.insert(index) {
                return Err(format!("duplicate expectation selector: {}", text));
            }
            selected_indexes.push(index);
        }
    }

    selected_indexes
        .into_iter()
        .map(|index| selected_expectation_at(config, identities, index, true))
        .collect::<Result<Vec<_>, _>>()
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
        .map(|expectation| expectation_id(&expectation.q))
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

fn minimal_unique_expectation_prefix(id: &str, ids: &[String]) -> Option<String> {
    (1..=id.len()).find_map(|end| {
        let prefix = &id[..end];
        let matches = ids
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .count();
        (matches == 1).then(|| prefix.to_string())
    })
}
