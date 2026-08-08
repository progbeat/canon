use super::{selected_expectation_at, ExpectationIdentity};
use crate::check::core::ResolvedExpectation;
use crate::config_types::CheckConfig;
use std::collections::BTreeSet;
use std::ffi::OsString;

const EXCLUSION_SELECTOR_PREFIX: &str = "not:";
const EXPECTATION_SELECTION_FAILED: &str = "expectation selection could not be completed";

pub(crate) fn select_expectations_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    args: &[OsString],
) -> Result<Vec<ResolvedExpectation>, String> {
    let mut selected_indexes = Vec::new();
    if args.is_empty() {
        selected_indexes.extend(0..config.expectations.len());
    } else {
        let selectors = parse_expectation_selectors(args)?;
        let has_include = selectors
            .iter()
            .any(|selector| matches!(selector, ExpectationSelector::Include(_)));
        // Resolve every exclusion before includes. In dynamic show, the
        // appended current-expectation exclusion therefore removes that
        // identity from prefix matching as well as from rendered output.
        // Unknown exclusions match nothing. When exclusions overlap, indexes
        // already contributed by an earlier prefix have no additional effect;
        // each prefix still contributes every new identity that it matches.
        let mut excluded_indexes = BTreeSet::new();
        for selector in &selectors {
            if let ExpectationSelector::Exclude(text) = selector {
                excluded_indexes.extend(matching_expectation_indexes(identities, text));
            }
        }
        if !has_include {
            selected_indexes.extend(
                (0..config.expectations.len()).filter(|index| !excluded_indexes.contains(index)),
            );
        }
        let mut seen_included = BTreeSet::new();
        for selector in selectors {
            match selector {
                ExpectationSelector::Include(text) => {
                    let matches = matching_expectation_indexes(identities, &text)
                        .into_iter()
                        .filter(|index| !excluded_indexes.contains(index))
                        .collect::<Vec<_>>();
                    if matches.is_empty() {
                        return Err(EXPECTATION_SELECTION_FAILED.to_string());
                    }
                    for index in matches {
                        if seen_included.insert(index) {
                            selected_indexes.push(index);
                        }
                    }
                }
                ExpectationSelector::Exclude(_) => {}
            }
        }
    }

    selected_indexes
        .into_iter()
        .map(|index| selected_expectation_at(config, identities, index))
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
        if let Some(excluded) = text.strip_prefix(EXCLUSION_SELECTOR_PREFIX) {
            if excluded.is_empty() {
                return Err("expectation selector must not be empty".to_string());
            }
            if !seen.insert(text.to_string()) {
                continue;
            }
            selectors.push(ExpectationSelector::Exclude(excluded.to_string()));
        } else {
            if !seen.insert(text.to_string()) {
                continue;
            }
            selectors.push(ExpectationSelector::Include(text.to_string()));
        }
    }
    Ok(selectors)
}

fn matching_expectation_indexes(identities: &[ExpectationIdentity], selector: &str) -> Vec<usize> {
    identities
        .iter()
        .enumerate()
        // Selectors are prefixes of the full expectation ID; a display ID is
        // only the shortest such prefix that is unique for human-facing output.
        .filter_map(|(index, identity)| identity.id.starts_with(selector).then_some(index))
        .collect()
}
