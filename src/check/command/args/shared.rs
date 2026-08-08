use crate::check::cli_args::value_arg;
use crate::check::CHECK_PATH;
use crate::git::{validate_tree_arg, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::notes::arg_to_string;
use crate::scope::normalize_repo_path;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

const NO_SANDBOX_ENV: &str = "CANON_NO_SANDBOX";

pub(super) struct SharedCommandArgs {
    pub(super) config_path: PathBuf,
    pub(super) config_explicit: bool,
    pub(super) tree: String,
    pub(super) tree_explicit: bool,
    pub(super) against_tree: String,
    pub(super) against_tree_explicit: bool,
    pub(super) in_place: bool,
    pub(super) no_sandbox: bool,
}

pub(super) fn parse_command_matches(
    command: Command,
    args: &[OsString],
) -> Result<ArgMatches, String> {
    command
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())
}

pub(super) fn parse_shared_command_args(
    matches: &ArgMatches,
    default_in_place: bool,
    no_sandbox: bool,
) -> Result<SharedCommandArgs, String> {
    let config_explicit = matches.contains_id("config");
    let config_path = matches
        .get_one::<OsString>("config")
        .map(|value| normalize_check_config_path(&arg_to_string(value)?))
        .transpose()?
        .unwrap_or_else(|| PathBuf::from(CHECK_PATH));
    let (tree, tree_explicit) = parse_tree_value(matches, "tree", "--tree", STAGED_TREE_ARG)?;
    let (against_tree, against_tree_explicit) = parse_tree_value(
        matches,
        "against_tree",
        "--against-tree",
        DEFAULT_AGAINST_TREE_ARG,
    )?;
    Ok(SharedCommandArgs {
        config_path,
        config_explicit,
        tree,
        tree_explicit,
        against_tree,
        against_tree_explicit,
        in_place: default_in_place || matches.get_flag("in_place"),
        no_sandbox,
    })
}

fn parse_tree_value(
    matches: &ArgMatches,
    id: &str,
    option: &str,
    default: &str,
) -> Result<(String, bool), String> {
    let explicit = matches.contains_id(id);
    let value = match matches.get_one::<OsString>(id) {
        Some(value) => {
            let value = arg_to_string(value)?;
            validate_tree_arg(&value, option)?;
            value
        }
        None => default.to_string(),
    };
    Ok((value, explicit))
}

pub(super) fn command_with_shared_args(
    command: Command,
    tree_help: &'static str,
    in_place_help: &'static str,
) -> Command {
    command
        .arg(
            value_arg("config")
                .short('c')
                .long("config")
                .value_name("PATH")
                .help("Read expectations from this config file [default: .canon/check.yml]"),
        )
        .arg(tree_arg(tree_help))
        .arg(against_tree_arg())
        .arg(
            Arg::new("in_place")
                .long("in-place")
                .help(in_place_help)
                .action(ArgAction::SetTrue),
        )
}

pub(super) fn no_sandbox_arg() -> Arg {
    Arg::new("no_sandbox")
        .long("no-sandbox")
        .env(NO_SANDBOX_ENV)
        .help("Disable canon-managed sandboxing; caller is responsible for isolation")
        .action(ArgAction::SetTrue)
}

pub(super) fn no_sandbox_from_env() -> Result<bool, String> {
    parse_no_sandbox_env(std::env::var_os(NO_SANDBOX_ENV).as_deref())
}

pub(super) fn parse_no_sandbox_env(value: Option<&OsStr>) -> Result<bool, String> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("{} must be valid UTF-8", NO_SANDBOX_ENV))?;
    value
        .parse::<bool>()
        .map_err(|_| format!("{} must be `true` or `false`", NO_SANDBOX_ENV))
}

fn tree_arg(help: &'static str) -> Arg {
    value_arg("tree").long("tree").value_name("TREE").help(help)
}

fn against_tree_arg() -> Arg {
    value_arg("against_tree")
        .long("against-tree")
        .value_name("TREE")
        .help("Compare against this Git tree [default: HEAD]")
}

fn normalize_check_config_path(value: &str) -> Result<PathBuf, String> {
    let normalized = normalize_repo_path(value).map_err(|err| format!("--config path: {}", err))?;
    if normalized == "." {
        return Err("--config path must name a file".to_string());
    }
    Ok(PathBuf::from(normalized))
}

pub(super) fn validate_in_place_options(
    command_name: &str,
    tree_explicit: bool,
    against_tree_explicit: bool,
) -> Result<(), String> {
    let mut invalid = Vec::new();
    if tree_explicit {
        invalid.push("--tree");
    }
    if against_tree_explicit {
        invalid.push("--against-tree");
    }
    if invalid.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} --in-place cannot be combined with {}",
        command_name,
        invalid.join(", ")
    ))
}
