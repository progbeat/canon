use crate::check::core::{CheckCommandArgs, RawCheckOptions};
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
    let query_scope_provided = !query_scope.is_empty();
    let options = raw_check_options_from_matches(&matches)?;
    let in_place = default_in_place || matches.get_flag("in_place");

    if in_place {
        // This parser rejects CLI options whose meaning depends on a Git tree
        // or caller-provided query scope. Expectation-level in-place
        // validation for diff-from/target/cooldown/ignore happens after config
        // expansion in `src/check/command/execution/in_place.rs`, and
        // generators/includes are expanded before that by `repo_inspection`.
        validate_in_place_options(tree_explicit, against_tree_explicit, query_scope_provided)?;
    }

    if query.is_none() && !query_scope.is_empty() {
        return Err("canon check -s/--scope requires -q".to_string());
    }
    if query.is_none() && query_preset.is_some() {
        return Err("canon check --preset requires -q".to_string());
    }
    if query.is_some() {
        validate_query_mode_options(&options)?;
    }
    if query.is_some() && query_scope.is_empty() {
        query_scope = full_scope();
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        tree,
        against_tree,
        against_tree_explicit,
        in_place,
        no_sandbox: matches.get_flag("no_sandbox"),
        query,
        query_preset,
        query_scope,
        query_scope_provided,
        options,
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
        .arg(checked_tree_arg())
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
            "Examples:\n  canon check\n      Check staged content against all canon expectations.\n\n  canon check a7F K9m\n      Check canon expectations selected by ID prefix.\n\n  canon check not:a7F not:K9m\n      Check all expectations except those whose IDs start with a7F or K9m.\n\n  canon check --tree HEAD --against-tree HEAD~1 a7F\n      Check one canon expectation on HEAD with comparison against the previous commit.\n\n  canon check -q \"Does the app expose Undo?\"\n      Ask a one-off question.\n\n  canon check -q \"Does the app expose Undo?\" -s src/app.rs\n      Ask a one-off question with a restricted visible scope.",
        )
}

fn check_value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

fn checked_tree_arg() -> Arg {
    check_value_arg("tree")
        .long("tree")
        .value_name("TREE")
        .help("Check this Git tree [default: :staged]")
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

fn validate_query_mode_options(options: &RawCheckOptions) -> Result<(), String> {
    let mut invalid = Vec::new();
    if !options.selectors.is_empty() {
        invalid.push("expectation selectors");
    }
    if options.keep_going {
        invalid.push("--keep-going");
    }
    if options.ignore_cooldown {
        invalid.push("--ignore-cooldown");
    }
    if invalid.is_empty() {
        return Ok(());
    }
    Err(format!(
        "canon check -q cannot be combined with {}",
        invalid.join(", ")
    ))
}

fn validate_in_place_options(
    tree_explicit: bool,
    against_tree_explicit: bool,
    query_scope_provided: bool,
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
    if invalid.is_empty() {
        return Ok(());
    }
    Err(format!(
        "canon check --in-place cannot be combined with {}",
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
    fn query_accepts_preset() {
        let command = parse(&["-q", "Can this pass?", "--preset", "smart"]).unwrap();

        assert_eq!(command.query.as_deref(), Some("Can this pass?"));
        assert_eq!(command.query_preset.as_deref(), Some("smart"));
    }

    #[test]
    fn query_defaults_to_full_scope() {
        let command = parse(&["-q", "Can this pass?"]).unwrap();

        assert_eq!(command.query_scope, vec![".".to_string()]);
        assert!(!command.query_scope_provided);
    }

    #[test]
    fn query_tracks_explicit_scope() {
        let command = parse(&["-q", "Can this pass?", "-s", "."]).unwrap();

        assert_eq!(command.query_scope, vec![".".to_string()]);
        assert!(command.query_scope_provided);
    }

    #[test]
    fn query_preserves_no_sandbox_flag() {
        let command = parse(&["-q", "Can this pass?", "--no-sandbox"]).unwrap();

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
    fn in_place_rejects_query_scope() {
        let err = match parse(&["--in-place", "-q", "Can this pass?", "-s", "src"]) {
            Ok(_) => panic!("expected in-place query scope to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon check --in-place cannot be combined with -s/--scope"
        );
    }

    #[test]
    fn query_accepts_break_after_tokens() {
        // `--break-after-tokens` is a hidden internal/test control. Accepting
        // it here does not make it part of the documented public help surface.
        let command = parse(&["-q", "Can this pass?", "--break-after-tokens", "1"]).unwrap();

        assert_eq!(command.options.break_after_tokens, Some(1));
    }

    #[test]
    fn query_rejects_expectation_selectors() {
        let err = match parse(&["-q", "Can this pass?", "abc"]) {
            Ok(_) => panic!("expected query selector combination to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon check -q cannot be combined with expectation selectors"
        );
    }

    #[test]
    fn query_rejects_ignored_check_run_options() {
        // `--ignore-cooldown` is hidden from `canon check --help`; this test
        // only verifies that query mode rejects it when supplied explicitly.
        let err = match parse(&["-q", "Can this pass?", "--keep-going", "--ignore-cooldown"]) {
            Ok(_) => panic!("expected query check-run options to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "canon check -q cannot be combined with --keep-going, --ignore-cooldown"
        );
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
