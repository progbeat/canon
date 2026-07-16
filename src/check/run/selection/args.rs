use super::identity::{select_expectations_with_identities, ExpectationIdentity};
use crate::check::core::{CheckOptions, RawCheckOptions};
use crate::config_types::CheckConfig;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::ffi::OsString;

pub(crate) fn resolve_check_options_with_identities(
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    options: &RawCheckOptions,
) -> Result<CheckOptions, String> {
    let candidates = select_expectations_with_identities(config, identities, &options.selectors)?;
    Ok(CheckOptions {
        candidate_expectations: candidates,
        selectors_provided: !options.selectors.is_empty(),
        keep_going: options.keep_going,
        break_after_tokens: options.break_after_tokens,
    })
}

pub(crate) fn add_check_option_args(command: Command) -> Command {
    command
        .arg(
            Arg::new("keep_going")
                .long("keep-going")
                .help("Continue after failures")
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
                .value_name("SELECTOR")
                .help("Expectation selectors: <ID-PREFIX> or not:<ID-PREFIX>")
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
