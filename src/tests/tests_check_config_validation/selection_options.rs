use crate::check_selection::{expectation_identities, parse_check_options, select_expectations};
use crate::tests::{check_config_yaml, parse_check_config, test_selector};

#[test]
fn selected_expectation_selectors_are_validated() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let second_id = test_selector(&config, "2");
    let second_prefix = expectation_identities(&config).unwrap()[1]
        .display_id
        .clone();
    assert_eq!(select_expectations(&config, &[]).unwrap().len(), 2);
    assert_eq!(
        select_expectations(&config, &[second_id.into()]).unwrap()[0].number,
        2
    );
    assert_eq!(
        select_expectations(&config, &[second_prefix.into()]).unwrap()[0].number,
        2
    );
    assert!(select_expectations(&config, &["".into()]).is_err());
    assert!(select_expectations(&config, &["not-a-prefix".into()]).is_err());
    let duplicate = test_selector(&config, "1");
    assert!(select_expectations(&config, &[duplicate.clone().into(), duplicate.into()]).is_err());
}

#[test]
fn check_options_accept_all_with_selected_selectors() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let default_options =
        parse_check_options(&config, &[test_selector(&config, "2").into()]).unwrap();
    assert!(!default_options.check_all);
    assert!(!default_options.ignore_cooldown);
    assert_eq!(default_options.break_after_tokens, None);
    assert_eq!(default_options.selected.len(), 1);
    assert_eq!(default_options.selected[0].number, 2);
    assert_eq!(default_options.skipped, 1);

    let all_options = parse_check_options(
        &config,
        &[
            "--all".into(),
            "--ignore-cooldown".into(),
            "--break-after-tokens".into(),
            "200000".into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    assert!(all_options.check_all);
    assert!(all_options.ignore_cooldown);
    assert_eq!(all_options.break_after_tokens, Some(200000));
    assert_eq!(all_options.selected.len(), 1);
    assert_eq!(all_options.selected[0].number, 2);
    assert_eq!(all_options.skipped, 1);
    assert!(parse_check_options(&config, &["--all".into(), "--all".into()]).is_err());
    assert!(parse_check_options(
        &config,
        &["--ignore-cooldown".into(), "--ignore-cooldown".into()]
    )
    .is_err());
    assert!(parse_check_options(&config, &["--break-after-tokens".into()]).is_err());
    assert!(parse_check_options(&config, &["--break-after-tokens".into(), "0".into()]).is_err());

    assert!(parse_check_options(&config, &["--all=true".into()]).is_err());
}

#[test]
fn check_options_stop_parsing_flags_after_double_dash() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let selector = test_selector(&config, "1");
    let options = parse_check_options(
        &config,
        &["--ignore-cache".into(), "--".into(), selector.into()],
    )
    .unwrap();
    assert!(options.ignore_cache);
    assert_eq!(options.selected.len(), 1);
    assert_eq!(options.selected[0].number, 1);

    let err = match parse_check_options(&config, &["--".into(), "--all".into()]) {
        Ok(_) => panic!("--all after -- must be parsed as a selector"),
        Err(err) => err,
    };
    assert!(err.contains("unknown expectation selector: --all"));
}
