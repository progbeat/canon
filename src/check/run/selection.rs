use crate::check::core::types::{
    CheckOptions, CheckResult, Cooldown, ObservedAnswerState, RawCheckOptions, SelectedExpectation,
};
use crate::check::run::order_state::latest_recorded_non_pass_timestamp_with_cache;
use crate::config_types::{CheckConfig, CooldownConfig};
use crate::hash::expectation_id;
use crate::history::HistoryCache;
use crate::time::parse_record_timestamp;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

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
        selectors_provided: !options.selectors.is_empty(),
        skipped,
        keep_going: options.keep_going,
        ignore_cooldown: options.ignore_cooldown,
        break_after_tokens: options.break_after_tokens,
    })
}

pub(crate) fn add_check_option_args(command: Command) -> Command {
    command
        .arg(
            Arg::new("keep_going")
                .long("keep-going")
                .alias("all")
                .help("Continue after failures")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("ignore_cooldown")
                .long("ignore-cooldown")
                .help("Re-evaluate expectations in cooldown")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("break_after_tokens")
                .long("break-after-tokens")
                .help("Stop after this many evaluator tokens")
                .hide(true)
                .num_args(1)
                .allow_hyphen_values(true)
                .value_parser(OsStringValueParser::new()),
        )
        .arg(
            Arg::new("selectors")
                .help("Expectation selectors: ID prefixes or full expectation IDs")
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
        keep_going: matches.get_flag("keep_going"),
        ignore_cooldown: matches.get_flag("ignore_cooldown"),
        break_after_tokens,
        selectors: matched_os_values(matches, "selectors"),
    })
}

pub(crate) fn matched_os_values(matches: &ArgMatches, id: &str) -> Vec<OsString> {
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

pub(crate) fn select_expectations_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    // This resolves command-line expectation selectors into the candidate set.
    // `check::run` always removes reusable cached results before evaluation
    // starts, so the final selected set contains only expectations that still
    // require evaluator work.
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

    // `display_id` is intentionally computed from `identities`, which cover
    // all collected expectations in the expanded config. Command-line
    // selectors only choose which collected expectations are processed; they do
    // not narrow the prefix namespace used by check output.
    selected_indexes
        .into_iter()
        .map(|index| selected_expectation_at(config, identities, index, true))
        .collect::<Result<Vec<_>, _>>()
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
        q: expectation.q.clone(),
        a: expectation.a.clone(),
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

pub(crate) fn order_expectations_by_latest_non_pass(
    root: &Path,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
) -> Result<Vec<SelectedExpectation>, String> {
    let mut ordered = selected
        .into_iter()
        .enumerate()
        .map(|(index, expectation)| {
            let latest = latest_non_pass_timestamp_with_cache(root, &expectation, history_cache)?;
            Ok(OrderedExpectation {
                expectation,
                latest,
                index,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|left, right| {
        right
            .latest
            .cmp(&left.latest)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(ordered
        .into_iter()
        .map(|ordered| ordered.expectation)
        .collect())
}

struct OrderedExpectation {
    expectation: SelectedExpectation,
    latest: u64,
    index: usize,
}

fn latest_history_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<u64>, String> {
    // This reads answer-history cache records, not runtime logs. Runtime logs
    // are diagnostic output and must not feed selection/order behavior. Stored
    // answer-history result metadata is not authoritative; the Cache spec says
    // the current result is derived from observed vs the current expected
    // answer.
    Ok(history_cache
        .read_records(root, expectation)?
        .into_iter()
        .filter(|record| history_record_is_non_pass(record, &expectation.a))
        .filter_map(|record| parse_record_timestamp(&record.timestamp))
        .max())
}

fn history_record_is_non_pass(
    record: &crate::check::core::types::CheckRecord,
    expected: &str,
) -> bool {
    ObservedAnswerState::from_error(record.error.as_deref()).requires_human_review()
        || CheckResult::from_expected_answer(expected, &record.observed) == CheckResult::Fail
}

pub(crate) fn latest_non_pass_timestamp_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<u64, String> {
    let latest = latest_history_non_pass_timestamp(root, expectation, history_cache)?
        .into_iter()
        .chain(latest_recorded_non_pass_timestamp_with_cache(
            root,
            expectation,
            history_cache,
        )?)
        .max();
    Ok(latest.unwrap_or(UNIX_EPOCH_TIMESTAMP))
}

pub(crate) fn parse_cooldown(value: &CooldownConfig) -> Result<Cooldown, String> {
    match value {
        CooldownConfig::Compact(value) => Ok(Cooldown {
            pass_seconds: Some(parse_cooldown_duration(value)?),
            fail_seconds: None,
        }),
        CooldownConfig::Mapping(mapping) => {
            if mapping.pass.is_none() && mapping.fail.is_none() {
                return Err("mapping must contain pass or fail".to_string());
            }
            Ok(Cooldown {
                pass_seconds: mapping
                    .pass
                    .as_deref()
                    .map(parse_cooldown_duration)
                    .transpose()
                    .map_err(|err| format!("pass: {}", err))?,
                fail_seconds: mapping
                    .fail
                    .as_deref()
                    .map(parse_cooldown_duration)
                    .transpose()
                    .map_err(|err| format!("fail: {}", err))?,
            })
        }
    }
}

fn parse_cooldown_duration(value: &str) -> Result<u64, String> {
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
    Ok(seconds)
}
