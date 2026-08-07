mod ask;
mod check;
mod shared;

pub(crate) use ask::{ask_help_command, parse_ask_command_args};
pub(crate) use check::{check_help_command, parse_check_command_args};

#[cfg(test)]
use crate::check::core::{AskCommandArgs, CheckCommandArgs};
#[cfg(test)]
use std::ffi::OsString;
#[cfg(test)]
use std::path::PathBuf;

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

    fn check_error(result: Result<CheckCommandArgs, String>) -> String {
        match result {
            Ok(_) => panic!("expected check arguments to fail"),
            Err(error) => error,
        }
    }

    fn ask_error(result: Result<AskCommandArgs, String>) -> String {
        match result {
            Ok(_) => panic!("expected ask arguments to fail"),
            Err(error) => error,
        }
    }

    #[test] // xpec: kK,sw
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

    #[test] // xpec: kK,l
    fn check_rejects_old_query_flag() {
        let err = match parse(&["-q", "Can this pass?"]) {
            Ok(_) => panic!("expected old query flag to fail"),
            Err(err) => err,
        };
        assert!(err.contains("unexpected argument"));
    }

    #[test] // xpec: kK,l
    fn check_help_excludes_ask_only_options() {
        let mut help = Vec::new();
        check_help_command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        assert!(!help.contains("-q"));
        assert!(!help.contains("--preset"));
        assert!(!help.contains("--scope"));
    }

    #[test] // xpec: l,nK
    fn ask_accepts_preset() {
        let command = parse_ask(&["Can this pass?", "--preset", "smart"]).unwrap();
        assert_eq!(command.question, "Can this pass?");
        assert_eq!(command.default_agent_preset.as_deref(), Some("smart"));
    }

    #[test] // xpec: l,nK
    fn ask_tracks_explicit_config() {
        let command = parse_ask(&["Can this pass?", "--config", "custom.yml"]).unwrap();
        assert_eq!(command.config_path, PathBuf::from("custom.yml"));
        assert!(command.config_explicit);
    }

    #[test] // xpec: l,qv
    fn ask_preserves_existing_scope_option_rejection() {
        let error = ask_error(parse_ask(&["Can this pass?", "--scope", "src"]));
        assert!(error.contains("unexpected argument"));
    }

    #[test] // xpec: l,nK,hQ
    fn ask_preserves_check_only_no_sandbox_rejection() {
        let error = ask_error(parse_ask(&["Can this pass?", "--no-sandbox"]));
        assert!(error.contains("unexpected argument"));
    }

    #[test] // xpec: l,hQ
    fn external_sandbox_environment_uses_a_boolean_contract() {
        use std::ffi::OsStr;

        assert!(!shared::parse_no_sandbox_env(None).unwrap());
        assert!(shared::parse_no_sandbox_env(Some(OsStr::new("true"))).unwrap());
        assert!(!shared::parse_no_sandbox_env(Some(OsStr::new("false"))).unwrap());
        assert!(shared::parse_no_sandbox_env(Some(OsStr::new("invalid"))).is_err());
    }

    #[test] // xpec: 90
    fn in_place_flag_is_recorded() {
        assert!(parse(&["--in-place"]).unwrap().in_place);
    }

    #[test] // xpec: kK
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

    #[test] // xpec: kK
    fn non_default_source_value_is_not_a_command_default() {
        assert!(
            !parse(&["--tree", "HEAD"])
                .unwrap()
                .sources_have_command_default_values
        );
    }

    #[test] // xpec: 90
    fn default_in_place_is_recorded() {
        assert!(parse_default_in_place(&[]).unwrap().in_place);
    }

    #[test] // xpec: 90
    fn in_place_rejects_git_tree_options() {
        let err = check_error(parse(&[
            "--in-place",
            "--tree",
            "HEAD",
            "--against-tree",
            "HEAD~1",
        ]));
        assert_eq!(
            err,
            "canon check --in-place cannot be combined with --tree, --against-tree"
        );
    }

    #[test] // xpec: 90
    fn default_in_place_rejects_git_tree_options() {
        let err = check_error(parse_default_in_place(&["--tree", "HEAD"]));
        assert_eq!(err, "canon check --in-place cannot be combined with --tree");
    }

    #[test] // xpec: l
    fn ask_preserves_empty_question() {
        assert_eq!(parse_ask(&[""]).unwrap().question, "");
    }

    #[test] // xpec: kK,l
    fn ask_rejects_check_run_options() {
        let err = ask_error(parse_ask(&["Can this pass?", "--keep-going"]));
        assert!(err.contains("unexpected argument"));
    }

    #[test] // xpec: l,nK
    fn preset_name_must_not_be_empty() {
        let err = ask_error(parse_ask(&["Can this pass?", "--preset", ""]));
        assert_eq!(err, "--preset name must not be empty");
    }
}
