use crate::check::core::{AskCommandArgs, CheckCommandArgs};
use crate::check::run::selection::{add_check_option_args, raw_check_options_from_matches};
use crate::check::CHECK_PATH;
use crate::git::{validate_tree_arg, DEFAULT_AGAINST_TREE_ARG, STAGED_TREE_ARG};
use crate::notes::arg_to_string;
use crate::scope::normalize_repo_path;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

struct SharedCommandArgs {
    config_path: PathBuf,
    config_explicit: bool,
    tree: String,
    tree_explicit: bool,
    against_tree: String,
    against_tree_explicit: bool,
    in_place: bool,
    no_sandbox: bool,
}

pub(crate) fn parse_check_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<CheckCommandArgs, String> {
    let matches = parse_command_matches(check_help_command(), args)?;
    let shared = parse_shared_command_args(&matches, default_in_place, true)?;
    let options = raw_check_options_from_matches(&matches)?;
    if shared.in_place {
        // This parser rejects CLI options whose meaning depends on a Git tree
        // or path-hiding behavior. Config-level mode compatibility runs after
        // raw config expansion, so generators/includes have already resolved.
        validate_in_place_options(
            "canon check",
            shared.tree_explicit,
            shared.against_tree_explicit,
        )?;
    }

    Ok(CheckCommandArgs {
        sources_have_command_default_values: shared.config_path == Path::new(CHECK_PATH)
            && shared.tree == STAGED_TREE_ARG
            && shared.against_tree == DEFAULT_AGAINST_TREE_ARG,
        config_path: shared.config_path,
        tree: shared.tree,
        against_tree: shared.against_tree,
        in_place: shared.in_place,
        no_sandbox: shared.no_sandbox,
        options,
    })
}

pub(crate) fn parse_ask_command_args(
    args: &[OsString],
    default_in_place: bool,
) -> Result<AskCommandArgs, String> {
    let matches = parse_command_matches(ask_help_command(), args)?;
    let shared = parse_shared_command_args(&matches, default_in_place, false)?;
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

    if shared.in_place {
        validate_in_place_options(
            "canon ask",
            shared.tree_explicit,
            shared.against_tree_explicit,
        )?;
    }

    Ok(AskCommandArgs {
        config_path: shared.config_path,
        config_explicit: shared.config_explicit,
        tree: shared.tree,
        against_tree: shared.against_tree,
        in_place: shared.in_place,
        question,
        default_agent_preset,
    })
}

fn parse_command_matches(command: Command, args: &[OsString]) -> Result<ArgMatches, String> {
    command
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())
}

fn parse_shared_command_args(
    matches: &ArgMatches,
    default_in_place: bool,
    supports_no_sandbox: bool,
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
        no_sandbox: supports_no_sandbox && matches.get_flag("no_sandbox"),
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

pub(crate) fn check_help_command() -> Command {
    let command = command_with_shared_args(
        Command::new("check")
            .bin_name("canon check")
            .about("Check whether project files meet human expectations written in the canon."),
        "Check this Git tree [default: :staged]",
        "Check the current directory directly",
    )
    .arg(
        // `--no-sandbox` is part of the documented `canon check` surface;
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
    command_with_shared_args(
        Command::new("ask")
            .bin_name("canon ask")
            .about("Ask one canon-style question about the project."),
        "Ask against this Git tree [default: :staged]",
        "Ask in the current directory directly",
    )
    .arg(
        check_value_arg("preset")
            .long("preset")
            .value_name("PRESET")
            .help("Select a preset by name for the question [default: default]"),
    )
    .arg(
        check_value_arg("question")
            .value_name("QUESTION")
            .help("Question to ask")
            .required(true),
    )
    .after_help(
        "Examples:\n  canon ask \"Does the app expose Undo?\"\n      Ask a one-off question.",
    )
}

fn command_with_shared_args(
    command: Command,
    tree_help: &'static str,
    in_place_help: &'static str,
) -> Command {
    command
        .arg(
            check_value_arg("config")
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

fn check_value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

fn tree_arg(help: &'static str) -> Arg {
    check_value_arg("tree")
        .long("tree")
        .value_name("TREE")
        .help(help)
}

fn against_tree_arg() -> Arg {
    check_value_arg("against_tree")
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

fn validate_in_place_options(
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

    #[test] // xpec: 9b,sw
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

    #[test] // xpec: 9b,Ky
    fn check_rejects_old_query_flag() {
        let err = match parse(&["-q", "Can this pass?"]) {
            Ok(_) => panic!("expected old query flag to fail"),
            Err(err) => err,
        };

        assert!(err.contains("unexpected argument"));
    }

    #[test] // xpec: 9b,Ky
    fn check_help_excludes_ask_only_options() {
        let mut help = Vec::new();
        check_help_command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(!help.contains("-q"));
        assert!(!help.contains("--preset"));
        assert!(!help.contains("--scope"));
    }

    #[test] // xpec: Ky,nK
    fn ask_accepts_preset() {
        let command = parse_ask(&["Can this pass?", "--preset", "smart"]).unwrap();

        assert_eq!(command.question, "Can this pass?");
        assert_eq!(command.default_agent_preset.as_deref(), Some("smart"));
    }

    #[test] // xpec: Ky,nK
    fn ask_tracks_explicit_config() {
        let command = parse_ask(&["Can this pass?", "--config", "custom.yml"]).unwrap();

        assert_eq!(command.config_path, PathBuf::from("custom.yml"));
        assert!(command.config_explicit);
    }

    #[test] // xpec: 3i5,Nt
    fn ask_rejects_scope_option() {
        let error = match parse_ask(&["Can this pass?", "--scope", "src"]) {
            Ok(_) => panic!("expected ask scope to be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("unexpected argument"));
    }

    #[test] // xpec: 3i5,nK
    fn ask_rejects_check_only_no_sandbox_flag() {
        let error = match parse_ask(&["Can this pass?", "--no-sandbox"]) {
            Ok(_) => panic!("expected ask to reject check-only --no-sandbox"),
            Err(error) => error,
        };

        assert!(error.contains("unexpected argument"));
    }

    #[test] // xpec: I4
    fn in_place_flag_is_recorded() {
        let command = parse(&["--in-place"]).unwrap();

        assert!(command.in_place);
    }

    #[test] // xpec: 7N
    fn explicit_default_source_values_remain_command_defaults() {
        let command = parse(&[
            "--config",
            "./.canon/check.yml",
            "--tree",
            ":staged",
            "--against-tree",
            "HEAD",
        ])
        .unwrap();

        assert!(command.sources_have_command_default_values);
    }

    #[test] // xpec: 7N
    fn non_default_source_value_is_not_a_command_default() {
        let command = parse(&["--tree", "HEAD"]).unwrap();

        assert!(!command.sources_have_command_default_values);
    }

    #[test] // xpec: I4
    fn default_in_place_is_recorded() {
        let command = parse_default_in_place(&[]).unwrap();

        assert!(command.in_place);
    }

    #[test] // xpec: I4
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

    #[test] // xpec: I4
    fn default_in_place_rejects_git_tree_options() {
        let err = match parse_default_in_place(&["--tree", "HEAD"]) {
            Ok(_) => panic!("expected default in-place tree option to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "canon check --in-place cannot be combined with --tree");
    }

    #[test] // xpec: Ky
    fn ask_preserves_empty_question() {
        let command = parse_ask(&[""]).unwrap();

        assert_eq!(command.question, "");
    }

    #[test] // xpec: 9b,Ky
    fn ask_rejects_check_run_options() {
        let err = match parse_ask(&["Can this pass?", "--keep-going"]) {
            Ok(_) => panic!("expected ask check-run option to fail"),
            Err(err) => err,
        };

        assert!(err.contains("unexpected argument"));
    }

    #[test] // xpec: Ky,nK
    fn preset_name_must_not_be_empty() {
        let err = match parse_ask(&["Can this pass?", "--preset", ""]) {
            Ok(_) => panic!("expected empty --preset to fail"),
            Err(err) => err,
        };

        assert_eq!(err, "--preset name must not be empty");
    }
}
