use crate::check_selection::{add_check_option_args, raw_check_options_from_matches};
use crate::check_types::CheckCommandArgs;
use crate::notes_cli::arg_to_string;
use crate::scope::normalize_repo_path;
use crate::CHECK_PATH;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let matches = check_command_args_parser()
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;

    let mut config_path = None;
    if let Some(value) = matches.get_one::<OsString>("config") {
        set_check_config_path(&mut config_path, &arg_to_string(value)?)?;
    }

    let query = match matches.get_one::<OsString>("query") {
        Some(value) => {
            let value = arg_to_string(value)?;
            if value.trim().is_empty() {
                return Err("-q question must not be empty".to_string());
            }
            Some(value)
        }
        None => None,
    };

    let mut query_scope = Vec::new();
    for value in matched_os_values(&matches, "scope") {
        let value = arg_to_string(&value)?;
        query_scope.push(normalize_query_scope_path("--scope", &value)?);
    }
    let options = raw_check_options_from_matches(&matches)?;

    if query.is_none() && !query_scope.is_empty() {
        return Err("canon check -s/--scope requires -q".to_string());
    }
    if query.is_some() && !options.is_empty() {
        return Err(
            "canon check -q cannot be combined with expectation selectors, --all, or --ignore-cache"
                .to_string(),
        );
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        query,
        query_scope,
        options,
    })
}

fn check_command_args_parser() -> Command {
    let command = Command::new("check")
        .no_binary_name(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(check_value_arg("config").short('c').long("config"))
        .arg(check_value_arg("query").short('q'))
        .arg(
            check_value_arg("scope")
                .short('s')
                .long("scope")
                .action(ArgAction::Append),
        );
    add_check_option_args(command)
}

fn matched_os_values(matches: &ArgMatches, id: &str) -> Vec<OsString> {
    matches
        .get_many::<OsString>(id)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn check_value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

fn set_check_config_path(config_path: &mut Option<PathBuf>, value: &str) -> Result<(), String> {
    if config_path.is_some() {
        return Err("duplicate --config".to_string());
    }
    *config_path = Some(normalize_check_config_path(value)?);
    Ok(())
}

fn normalize_check_config_path(value: &str) -> Result<PathBuf, String> {
    let normalized = normalize_repo_path(value).map_err(|err| format!("--config path: {}", err))?;
    if normalized == "." {
        return Err("--config path must name a file".to_string());
    }
    Ok(PathBuf::from(normalized))
}

fn normalize_query_scope_path(option: &str, value: &str) -> Result<String, String> {
    normalize_repo_path(value).map_err(|err| format!("{} path: {}", option, err))
}
