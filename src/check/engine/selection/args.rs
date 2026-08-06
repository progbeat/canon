use super::identity::{select_expectations_with_identities, ExpectationIdentity};
use crate::check::cli_args::expectation_selectors_arg;
use crate::check::core::{CheckOptions, RawCheckOptions};
use crate::config_types::CheckConfig;
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
        .arg(expectation_selectors_arg())
}

pub(crate) fn raw_check_options_from_matches(
    matches: &ArgMatches,
) -> Result<RawCheckOptions, String> {
    Ok(RawCheckOptions {
        keep_going: matches.get_flag("keep_going"),
        selectors: matched_os_values(matches, "selectors"),
    })
}

pub(crate) fn matched_os_values(matches: &ArgMatches, id: &str) -> Vec<OsString> {
    matches
        .get_many::<OsString>(id)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}
