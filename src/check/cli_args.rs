use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction};
use std::ffi::OsString;

const OPTIONS_WITH_HYPHEN_VALUES: [&str; 5] =
    ["-c", "--config", "--tree", "--against-tree", "--preset"];

pub(super) fn value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

pub(super) fn expectation_selectors_arg() -> Arg {
    Arg::new("selectors")
        .value_name("SELECTOR")
        .help("Expectation selectors: <ID-PREFIX> or not:<ID-PREFIX>")
        .num_args(0..)
        .action(ArgAction::Append)
        .value_parser(OsStringValueParser::new())
}

/// Detects an actual `--in-place` option using the same hyphen-value boundary
/// as the check-like Clap commands.
pub(super) fn args_request_in_place(args: &[OsString]) -> bool {
    let mut preceding_option_consumes_value = false;
    for arg in args {
        if arg == "--" {
            break;
        }
        if preceding_option_consumes_value {
            preceding_option_consumes_value = false;
            continue;
        }
        if arg == "--in-place" {
            return true;
        }
        preceding_option_consumes_value = OPTIONS_WITH_HYPHEN_VALUES
            .iter()
            .any(|option| arg == option);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::args_request_in_place;
    use std::ffi::OsString;

    #[test] // xpec: gN,90
    fn in_place_probe_distinguishes_flags_from_hyphen_prefixed_values() {
        assert!(args_request_in_place(&[OsString::from("--in-place")]));
        for value_option in ["-c", "--config", "--tree", "--against-tree", "--preset"] {
            assert!(!args_request_in_place(&[
                OsString::from(value_option),
                OsString::from("--in-place"),
            ]));
        }
        assert!(!args_request_in_place(&[
            OsString::from("--"),
            OsString::from("--in-place"),
        ]));
    }
}
