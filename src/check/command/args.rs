use crate::check::core::CheckCommandArgs;
use crate::check::run::selection::{
    add_check_option_args, matched_os_values, raw_check_options_from_matches,
};
use crate::check::CHECK_PATH;
use crate::git::{validate_tree_arg, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::hash::full_scope;
use crate::notes::arg_to_string;
use crate::scope::normalize_repo_path;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, Command};
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let matches = check_help_command()
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;

    let mut config_path = None;
    if let Some(value) = matches.get_one::<OsString>("config") {
        set_check_config_path(&mut config_path, &arg_to_string(value)?)?;
    }
    let tree = match matches.get_one::<OsString>("tree") {
        Some(value) => {
            let value = arg_to_string(value)?;
            validate_tree_arg(&value, "--tree")?;
            value
        }
        None => STAGED_TREE_ARG.to_string(),
    };
    let against_tree_explicit = matches.contains_id("against_tree");
    let against_tree = match matches.get_one::<OsString>("against_tree") {
        Some(value) => {
            let value = arg_to_string(value)?;
            validate_tree_arg(&value, "--against-tree")?;
            value
        }
        None => DEFAULT_AGAINST_TREE_ARG.to_string(),
    };

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
    let query_preset = match matches.get_one::<OsString>("preset") {
        Some(value) => {
            let value = arg_to_string(value)?;
            if value.trim().is_empty() {
                return Err("--preset name must not be empty".to_string());
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
    if query.is_none() && query_preset.is_some() {
        return Err("canon check --preset requires -q".to_string());
    }
    if query.is_some() && !options.is_empty() {
        return Err(
            "canon check -q cannot be combined with expectation selectors, --keep-going, --all, or --ignore-cooldown"
                .to_string(),
        );
    }
    if query.is_some() && query_scope.is_empty() {
        query_scope = full_scope();
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        tree,
        against_tree,
        against_tree_explicit,
        no_sandbox: matches.get_flag("no_sandbox"),
        query,
        query_preset,
        query_scope,
        options,
    })
}

pub(crate) fn check_help_command() -> Command {
    let command = Command::new("check")
        .bin_name("canon check")
        .about("Check whether a Git tree meets project expectations written in the canon.")
        .arg(
            check_value_arg("config")
                .short('c')
                .long("config")
                .value_name("PATH")
                .help("Read expectations from this config file [default: .canon/check.yml]"),
        )
        .arg(
            check_value_arg("query")
                .short('q')
                .value_name("QUESTION")
                .help("Ask one question"),
        )
        .arg(
            check_value_arg("preset")
                .long("preset")
                .value_name("PRESET")
                .help("Select a preset by name for the question [default: default]"),
        )
        .arg(
            check_value_arg("scope")
                .short('s')
                .long("scope")
                .value_name("PATHSPEC")
                .help("Set the visible scope for the question")
                .action(ArgAction::Append),
        )
        .arg(
            check_value_arg("tree")
                .long("tree")
                .value_name("TREE")
                .help("Check this Git tree [default: :staged]"),
        )
        .arg(
            check_value_arg("against_tree")
                .long("against-tree")
                .value_name("TREE")
                .help("Compare against this Git tree [default: HEAD]"),
        )
        .arg(
            Arg::new("no_sandbox")
                .long("no-sandbox")
                .help("Disable canon-managed sandboxing; caller is responsible for isolation")
                .action(ArgAction::SetTrue),
        );
    add_check_option_args(command).after_help(
            "Examples:\n  canon check\n      Check staged content against all canon expectations.\n\n  canon check a7F K9m\n      Check canon expectations selected by ID prefix.\n\n  canon check --tree HEAD --against-tree HEAD~1 a7F\n      Check one canon expectation on HEAD with comparison against the previous commit.\n\n  canon check -q \"Does the app expose Undo?\"\n      Ask a one-off question.\n\n  canon check -q \"Does the app expose Undo?\" -s src/app.rs\n      Ask a one-off question with a restricted visible scope.",
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CheckCommandArgs, String> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        parse_check_command_args(&args)
    }

    #[test]
    fn query_accepts_preset() {
        let command = parse(&["-q", "Can this pass?", "--preset", "smart"]).unwrap();

        assert_eq!(command.query.as_deref(), Some("Can this pass?"));
        assert_eq!(command.query_preset.as_deref(), Some("smart"));
    }

    #[test]
    fn query_defaults_to_full_scope() {
        let command = parse(&["-q", "Can this pass?"]).unwrap();

        assert_eq!(command.query_scope, vec![".".to_string()]);
    }

    #[test]
    fn preset_requires_query() {
        let err = match parse(&["--preset", "smart"]) {
            Ok(_) => panic!("expected --preset without -q to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "canon check --preset requires -q");
    }

    #[test]
    fn preset_name_must_not_be_empty() {
        let err = match parse(&["-q", "Can this pass?", "--preset", ""]) {
            Ok(_) => panic!("expected empty --preset to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "--preset name must not be empty");
    }
}
