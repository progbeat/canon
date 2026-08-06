use crate::check::cli_args::{expectation_selectors_arg, value_arg};
use crate::git::{validate_tree_arg, STAGED_TREE_ARG};
use crate::notes::arg_to_string;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, Command};
use std::ffi::OsString;

pub(super) struct ShowCommandArgs {
    pub(super) tree: String,
    pub(super) selectors: Vec<OsString>,
    pub(super) pathspecs: Vec<String>,
}

pub(crate) fn show_help_command() -> Command {
    Command::new("show")
        .bin_name("canon show")
        .about("Show canon expectations.")
        .arg(value_arg("tree").long("tree").value_name("TREE").help(
            "Use this Git tree for expectation collection and pathspec filtering \
                     [default: :staged]",
        ))
        .arg(expectation_selectors_arg())
        .arg(
            Arg::new("pathspecs")
                .value_name("PATHSPEC")
                .help("Limit output to expectations affected by changes matching these pathspecs")
                .num_args(1..)
                .last(true)
                .action(ArgAction::Append)
                .value_parser(OsStringValueParser::new()),
        )
}

pub(super) fn parse_show_command_args(args: &[OsString]) -> Result<ShowCommandArgs, String> {
    let matches = show_help_command()
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;
    let tree = match matches.get_one::<OsString>("tree") {
        Some(value) => {
            let value = arg_to_string(value)?;
            validate_tree_arg(&value, "--tree")?;
            value
        }
        None => STAGED_TREE_ARG.to_string(),
    };
    let pathspecs = matches
        .get_many::<OsString>("pathspecs")
        .unwrap_or_default()
        .map(arg_to_string)
        .collect::<Result<Vec<_>, _>>()?;
    if pathspecs.iter().any(|pathspec| pathspec.is_empty()) {
        return Err("pathspec must not be empty".to_string());
    }
    Ok(ShowCommandArgs {
        tree,
        selectors: matches
            .get_many::<OsString>("selectors")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        pathspecs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 2gZ
    fn parse_show_splits_selectors_from_pathspecs_after_separator() {
        let parsed = parse_show_command_args(&[
            OsString::from("abc"),
            OsString::from("--"),
            OsString::from("src/lib.rs"),
        ])
        .unwrap();

        assert_eq!(parsed.selectors, vec![OsString::from("abc")]);
        assert_eq!(parsed.pathspecs, vec!["src/lib.rs".to_string()]);
    }

    #[test] // xpec: 2gZ
    fn parse_show_supports_pathspecs_without_selectors() {
        let parsed = parse_show_command_args(&[
            OsString::from("--"),
            OsString::from("src/lib.rs"),
            OsString::from("tests"),
        ])
        .unwrap();

        assert!(parsed.selectors.is_empty());
        assert_eq!(
            parsed.pathspecs,
            vec!["src/lib.rs".to_string(), "tests".to_string()]
        );
    }

    #[test] // xpec: 2gZ
    fn parse_show_accepts_separator_without_pathspecs() {
        let parsed = parse_show_command_args(&[OsString::from("--")]).unwrap();

        assert!(parsed.selectors.is_empty());
        assert!(parsed.pathspecs.is_empty());
    }
}
