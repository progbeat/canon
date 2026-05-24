use crate::check_order_state::latest_recorded_non_pass_timestamp_with_cache;
use crate::check_types::{CheckOptions, Cooldown, RawCheckOptions, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::hash::expectation_id;
use crate::history::HistoryCache;
use crate::history_reuse::cooldown_history_record;
use crate::time::parse_record_timestamp;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

#[cfg(test)]
pub(crate) fn parse_check_options(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<CheckOptions, String> {
    let identities = expectation_identities(config)?;
    let options = parse_raw_check_options(args)?;
    resolve_check_options_with_identities(config, &identities, &options)
}

pub(crate) fn resolve_check_options_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    options: &RawCheckOptions,
) -> Result<CheckOptions, String> {
    let selected = select_expectations_with_identities(config, identities, &options.selectors)?;
    let non_selected =
        initial_non_selected_expectations_with_identities(config, identities, &selected)?;
    let skipped = config.expectations.len().saturating_sub(selected.len());
    Ok(CheckOptions {
        selected,
        non_selected,
        skipped,
        check_all: options.check_all,
        ignore_cache: options.ignore_cache,
        ignore_cooldown: options.ignore_cooldown,
        break_after_tokens: options.break_after_tokens,
    })
}

#[cfg(test)]
fn parse_raw_check_options(args: &[OsString]) -> Result<RawCheckOptions, String> {
    let matches = add_check_option_args(check_options_parser())
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;
    raw_check_options_from_matches(&matches)
}

#[cfg(test)]
fn check_options_parser() -> Command {
    Command::new("check-options")
        .no_binary_name(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
}

pub(crate) fn add_check_option_args(command: Command) -> Command {
    command
        .arg(Arg::new("all").long("all").action(ArgAction::SetTrue))
        .arg(
            Arg::new("ignore_cache")
                .long("ignore-cache")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("ignore_cooldown")
                .long("ignore-cooldown")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("break_after_tokens")
                .long("break-after-tokens")
                .num_args(1)
                .allow_hyphen_values(true)
                .value_parser(OsStringValueParser::new()),
        )
        .arg(
            Arg::new("selectors")
                .num_args(0..)
                .action(ArgAction::Append)
                .value_parser(OsStringValueParser::new()),
        )
}

pub(crate) fn raw_check_options_from_matches(
    matches: &ArgMatches,
) -> Result<RawCheckOptions, String> {
    let break_after_tokens = match matches.get_one::<OsString>("break_after_tokens") {
        Some(value) => {
            let value = value
                .to_str()
                .ok_or_else(|| "--break-after-tokens must be valid UTF-8".to_string())?;
            Some(parse_break_after_tokens(value)?)
        }
        None => None,
    };
    Ok(RawCheckOptions {
        check_all: matches.get_flag("all"),
        ignore_cache: matches.get_flag("ignore_cache"),
        ignore_cooldown: matches.get_flag("ignore_cooldown"),
        break_after_tokens,
        selectors: matched_os_values(matches, "selectors"),
    })
}

fn matched_os_values(matches: &ArgMatches, id: &str) -> Vec<OsString> {
    matches
        .get_many::<OsString>(id)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn parse_break_after_tokens(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("--break-after-tokens must be a positive integer".to_string());
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "--break-after-tokens value is too large".to_string())?;
    if parsed == 0 {
        return Err("--break-after-tokens must be greater than zero".to_string());
    }
    Ok(parsed)
}

#[cfg(test)]
pub(crate) fn select_expectations(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    let identities = expectation_identities(config)?;
    select_expectations_with_identities(config, &identities, args)
}

pub(crate) fn select_expectations_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    // This expands command-line expectation selectors into the candidate set.
    // The final selected set is resolved later, after cooldown selection
    // filtering and reusable passing exact-cache deselection.
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

#[cfg(test)]
pub(crate) fn initial_non_selected_expectations(
    config: &CheckConfig,
    selected: &[SelectedExpectation],
) -> Result<Vec<SelectedExpectation>, String> {
    let identities = expectation_identities(config)?;
    initial_non_selected_expectations_with_identities(config, &identities, selected)
}

pub(crate) fn initial_non_selected_expectations_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    selected: &[SelectedExpectation],
) -> Result<Vec<SelectedExpectation>, String> {
    let selected_ids = selected
        .iter()
        .map(|expectation| expectation.id.clone())
        .collect::<BTreeSet<_>>();
    let mut non_selected = Vec::new();
    for index in 0..config.expectations.len() {
        let identity = identities
            .get(index)
            .ok_or_else(|| "expectation identity count mismatch".to_string())?;
        if !selected_ids.contains(&identity.id) {
            non_selected.push(selected_expectation_at(config, identities, index, false)?);
        }
    }
    Ok(non_selected)
}

#[derive(Debug, Clone)]
pub(crate) struct ExpectationIdentity {
    pub(crate) id: String,
    pub(crate) display_id: String,
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
            .as_deref()
            .map(parse_cooldown)
            .transpose()?
    } else {
        None
    };
    Ok(SelectedExpectation {
        number: index + 1,
        id: identity.id.clone(),
        display_id: identity.display_id.clone(),
        q: expectation.q.clone(),
        a: expectation.a.clone(),
        cooldown,
        thinking: expectation.thinking.clone(),
    })
}

pub(crate) fn expectation_identities(
    config: &CheckConfig,
) -> Result<Vec<ExpectationIdentity>, String> {
    let ids = config
        .expectations
        .iter()
        .map(|expectation| expectation_id(&expectation.q, &expectation.a))
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

pub(crate) struct FinalSelection {
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) skipped: Vec<SelectedExpectation>,
}

pub(crate) struct FinalSelectionError {
    pub(crate) error: String,
    pub(crate) skipped: Vec<SelectedExpectation>,
}

pub(crate) fn final_selected_expectations(
    root: &Path,
    agent: &AgentConfig,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<FinalSelection, FinalSelectionError> {
    // CLI selector filtering happens before this shared final-selection step.
    // Cooldown is a selection filter, not an answer-cache hit: a fresh latest
    // pass removes a matching expectation before exact-cache lookup and before
    // any evaluator result can be reused as the observed answer. `canon check`
    // uses this set for evaluator work; `canon gate` uses the same no-selector
    // final selected set as the input to its raw cache-comparison loop.
    let mut remaining = Vec::new();
    let mut skipped = Vec::new();
    for expectation in selected {
        match cooldown_history_record(root, agent, &expectation, history_cache, now) {
            Ok(None) => remaining.push(expectation),
            Ok(Some(_)) => skipped.push(expectation),
            Err(error) => return Err(FinalSelectionError { error, skipped }),
        }
    }
    Ok(FinalSelection {
        selected: remaining,
        skipped,
    })
}

pub(crate) fn order_expectations_by_latest_non_pass(
    root: &Path,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
) -> Result<Vec<SelectedExpectation>, String> {
    let mut ordered = selected
        .into_iter()
        .enumerate()
        .map(|(index, expectation)| {
            let latest = latest_history_non_pass_timestamp(root, &expectation, history_cache)?
                .into_iter()
                .chain(latest_recorded_non_pass_timestamp_with_cache(
                    root,
                    &expectation,
                    history_cache,
                )?)
                .max();
            Ok(OrderedExpectation {
                expectation,
                latest,
                index,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|left, right| match (left.latest, right.latest) {
        (Some(left_time), Some(right_time)) => right_time
            .cmp(&left_time)
            .then_with(|| left.index.cmp(&right.index)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.index.cmp(&right.index),
    });
    Ok(ordered
        .into_iter()
        .map(|ordered| ordered.expectation)
        .collect())
}

struct OrderedExpectation {
    expectation: SelectedExpectation,
    latest: Option<u64>,
    index: usize,
}

fn latest_history_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<u64>, String> {
    // This reads answer-history cache records, not runtime logs. Runtime logs
    // are diagnostic output and must not feed selection/order behavior.
    Ok(history_cache
        .read_records(root, expectation)?
        .into_iter()
        .filter(|record| !record.passed())
        .filter_map(|record| parse_record_timestamp(&record.timestamp))
        .max())
}

pub(crate) fn parse_cooldown(value: &str) -> Result<Cooldown, String> {
    if value.trim() != value {
        return Err("must use compact duration syntax without surrounding whitespace".to_string());
    }
    let Some((unit_index, unit)) = value.char_indices().next_back() else {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    };
    if unit_index == 0 {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    }
    let digits = &value[..unit_index];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must start with an integer".to_string());
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "duration integer is too large".to_string())?;
    if amount == 0 {
        return Err("must be greater than zero".to_string());
    }
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => return Err("unit must be one of s, m, h, d, or w".to_string()),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_string())?;
    Ok(Cooldown { seconds })
}
