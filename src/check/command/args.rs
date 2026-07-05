use crate::check::core::{AskCommandArgs, CheckCommandArgs, RawCheckOptions};
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

pub(crate) fn parse_check_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<CheckCommandArgs, String> {
    let matches = check_help_command()
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;

    let mut config_path = None;
    if let Some(value) = matches.get_one::<OsString>("config") {
        set_check_config_path(&mut config_path, &arg_to_string(value)?)?;
    }
    let tree_explicit = matches.contains_id("tree");
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

    let options = raw_check_options_from_matches(&matches)?;
    let in_place = default_in_place || matches.get_flag("in_place");

    if in_place {
        // This parser rejects CLI options whose meaning depends on a Git tree
        // or cache/path-hiding behavior. Config-level mode compatibility
        // runs after raw config expansion, so generators/includes have already
        // been resolved.
        validate_in_place_options(
            "canon check",
            tree_explicit,
            against_tree_explicit,
            false,
            &options,
        )?;
    }

    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        tree,
        against_tree,
        against_tree_explicit,
        in_place,
        no_sandbox: matches.get_flag("no_sandbox"),
        options,
    })
}

pub(crate) fn parse_ask_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<AskCommandArgs, String> {
    let matches = ask_help_command()
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;

    let mut config_path = None;
    if let Some(value) = matches.get_one::<OsString>("config") {
        set_check_config_path(&mut config_path, &arg_to_string(value)?)?;
    }
    let tree_explicit = matches.contains_id("tree");
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
    let question = match matches.get_one::<OsString>("question") {
        Some(value) => arg_to_string(value)?,
        None => return Err("question is required".to_string()),
    };
    let default_agent_preset = match matches.get_one::<OsString>("preset") {
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
    let query_scope_provided = !query_scope.is_empty();
    if query_scope.is_empty() {
        query_scope = full_scope();
    }
    let in_place = default_in_place || matches.get_flag("in_place");
    if in_place {
        validate_in_place_options(
            "canon ask",
            tree_explicit,
            against_tree_explicit,
            query_scope_provided,
            &RawCheckOptions::default(),
        )?;
    }

    Ok(AskCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        tree,
        against_tree,
        against_tree_explicit,
        in_place,
        no_sandbox: matches.get_flag("no_sandbox"),
        question,
        default_agent_preset,
        query_scope,
        query_scope_provided,
    })
}

pub(crate) fn check_help_command() -> Command {
    let command = Command::new("check")
        .bin_name("canon check")
        .about("Check whether project files meet human expectations written in the canon.")
        .arg(
            check_value_arg("config")
                .short('c')
                .long("config")
                .value_name("PATH")
                .help("Read expectations from this config file [default: .canon/check.yml]"),
        )
        .arg(check_tree_arg())
        .arg(against_tree_arg())
        .arg(
            Arg::new("in_place")
                .long("in-place")
                .help("Check the current directory directly")
                .action(ArgAction::SetTrue),
        )
        .arg(
            // `--no-sandbox` is part of the documented public help surface;
            // Docker uses it when the container provides isolation.
            Arg::new("no_sandbox")
                .long("no-sandbox")
                .help("Disable canon-managed sandboxing; caller is responsible for isolation")
                .action(ArgAction::SetTrue),
        );
    // `add_check_option_args` supplies the documented public `--keep-going`
    // option and the selector argument used by the examples below, including
    // `not:<ID-PREFIX>` exclusions. It also registers hidden internal controls
    // that are intentionally absent from `canon check --help`.
    add_check_option_args(command).after_help(
            "Examples:\n  canon check\n      Check staged content against all canon expectations.\n\n  canon check a7F K9m\n      Check canon expectations selected by ID prefix.\n\n  canon check not:a7F not:K9m\n      Check all expectations except those whose IDs start with a7F or K9m.\n\n  canon check --tree HEAD --against-tree HEAD~1 a7F\n      Check one canon expectation on HEAD with comparison against the previous commit.",
        )
}

pub(crate) fn ask_help_command() -> Command {
    Command::new("ask")
        .bin_name("canon ask")
        .about("Ask one canon-style question about the project.")
        .arg(
            check_value_arg("config")
                .short('c')
                .long("config")
                .value_name("PATH")
                .help("Read expectations from this config file [default: .canon/check.yml]"),
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
            check_value_arg("preset")
                .long("preset")
                .value_name("PRESET")
                .help("Select a preset by name for the question [default: default]"),
        )
        .arg(ask_tree_arg())
        .arg(against_tree_arg())
        .arg(
            Arg::new("in_place")
                .long("in-place")
                .help("Ask in the current directory directly")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("no_sandbox")
                .long("no-sandbox")
                .help("Disable canon-managed sandboxing; caller is responsible for isolation")
                .action(ArgAction::SetTrue),
        )
        .arg(
            check_value_arg("question")
                .value_name("QUESTION")
                .help("Question to ask")
                .required(true),
        )
        .after_help(
            "Examples:\n  canon ask \"Does the app expose Undo?\"\n      Ask a one-off question.\n\n  canon ask \"Does the app expose Undo?\" -s src/app.rs\n      Ask a one-off question with a restricted visible scope.",
        )
}

fn check_value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

fn check_tree_arg() -> Arg {
    check_value_arg("tree")
        .long("tree")
        .value_name("TREE")
        .help("Check this Git tree [default: :staged]")
}

fn ask_tree_arg() -> Arg {
    check_value_arg("tree")
        .long("tree")
        .value_name("TREE")
        .help("Ask against this Git tree [default: :staged]")
}

fn against_tree_arg() -> Arg {
    check_value_arg("against_tree")
        .long("against-tree")
        .value_name("TREE")
        .help("Compare against this Git tree [default: HEAD]")
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

fn validate_in_place_options(
    command_name: &str,
    tree_explicit: bool,
    against_tree_explicit: bool,
    query_scope_provided: bool,
    options: &RawCheckOptions,
) -> Result<(), String> {
    let mut invalid = Vec::new();
    if tree_explicit {
        invalid.push("--tree");
    }
    if against_tree_explicit {
        invalid.push("--against-tree");
    }
    if query_scope_provided {
        invalid.push("-s/--scope");
    }
    if options.ignore_cooldown {
        invalid.push("--ignore-cooldown");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CheckCommandArgs, String> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        parse_check_command_args(&args, false)
    }

    fn parse_default_in_place(args: &[&str]) -> Result<CheckCommandArgs, String> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        parse_check_command_args(&args, true)
    }

    fn parse_ask(args: &[&str]) -> Result<AskCommandArgs, String> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        parse_ask_command_args(&args, false)
    }

    #[test]
    fn check_accepts_expectation_id_selectors() {
        let command = parse(&["a7F", "0123456789abcdefghij"]).unwrap();

        assert_eq!(
            command.options.selectors,
            vec![
                OsString::from("a7F"),
                OsString::from("0123456789abcdefghij")
            ]
        );
    }

    #[test]
    fn check_rejects_old_query_flag() {
        let err = match parse(&["-q", "Can this pass?"]) {
            Ok(_) => panic!("expected old query flag to fail"),
            Err(err) => err,
        };

        assert!(err.contains("unexpected argument"));
    }

    #[test] // xpec: HW
    fn check_help_excludes_ask_only_options() {
        let mut help = Vec::new();
        check_help_command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(!help.contains("-q"));
        assert!(!help.contains("--preset"));
        assert!(!help.contains("--scope"));
    }

    #[test]
    fn ask_accepts_preset() {
        let command = parse_ask(&["Can this pass?", "--preset", "smart"]).unwrap();

        assert_eq!(command.question, "Can this pass?");
        assert_eq!(command.default_agent_preset.as_deref(), Some("smart"));
    }

    #[test]
    fn ask_defaults_to_full_scope() {
        let command = parse_ask(&["Can this pass?"]).unwrap();

        assert_eq!(command.query_scope, vec![".".to_string()]);
        assert!(!command.query_scope_provided);
    }

    #[test]
    fn ask_tracks_explicit_scope() {
        let command = parse_ask(&["Can this pass?", "-s", "."]).unwrap();

        assert_eq!(command.query_scope, vec![".".to_string()]);
        assert!(command.query_scope_provided);
    }

    #[test]
    fn ask_preserves_no_sandbox_flag() {
        let command = parse_ask(&["Can this pass?", "--no-sandbox"]).unwrap();

        assert!(command.no_sandbox);
    }

    #[test]
    fn in_place_flag_is_recorded() {
        let command = parse(&["--in-place"]).unwrap();

        assert!(command.in_place);
    }

    #[test]
    fn default_in_place_is_recorded() {
        let command = parse_default_in_place(&[]).unwrap();

        assert!(command.in_place);
    }

    #[test]
    fn in_place_rejects_git_tree_options() {
        let err = match parse(&["--in-place", "--tree", "HEAD", "--against-tree", "HEAD~1"]) {
            Ok(_) => panic!("expected in-place tree options to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon check --in-place cannot be combined with --tree, --against-tree"
        );
    }

    #[test]
    fn default_in_place_rejects_git_tree_options() {
        let err = match parse_default_in_place(&["--tree", "HEAD"]) {
            Ok(_) => panic!("expected default in-place tree option to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "canon check --in-place cannot be combined with --tree");
    }

    #[test]
    fn in_place_ask_rejects_scope() {
        let err = match parse_ask(&["Can this pass?", "--in-place", "-s", "src"]) {
            Ok(_) => panic!("expected in-place ask scope to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon ask --in-place cannot be combined with -s/--scope"
        );
    }

    #[test]
    fn in_place_rejects_cache_controls() {
        let err = match parse(&["--in-place", "--ignore-cooldown"]) {
            Ok(_) => panic!("expected in-place cache control to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon check --in-place cannot be combined with --ignore-cooldown"
        );
    }

    #[test]
    fn git_backed_check_accepts_cache_controls() {
        let command = parse(&["--ignore-cooldown"]).unwrap();

        assert!(!command.in_place);
        assert!(command.options.ignore_cooldown);
    }

    #[test]
    fn ask_preserves_empty_question() {
        let command = parse_ask(&[""]).unwrap();

        assert_eq!(command.question, "");
    }

    #[test]
    fn ask_rejects_check_run_options() {
        let err = match parse_ask(&["Can this pass?", "--keep-going"]) {
            Ok(_) => panic!("expected ask check-run option to fail"),
            Err(err) => err,
        };

        assert!(err.contains("unexpected argument"));
    }

    #[test]
    fn preset_name_must_not_be_empty() {
        let err = match parse_ask(&["Can this pass?", "--preset", ""]) {
            Ok(_) => panic!("expected empty --preset to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "--preset name must not be empty");
    }
}
