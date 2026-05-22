use super::*;

#[test]
fn parser_handles_json_answer_and_free_form_evidence() {
    let parsed = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"line: one\nSCOPE: this is evidence\nANSWER: also evidence","scope":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap();
    assert_eq!(parsed.answer, "yes");
    assert_eq!(
        parsed.evidence,
        "line: one\nSCOPE: this is evidence\nANSWER: also evidence"
    );
    assert_eq!(parsed.scope, vec!["."]);
    let escaped_keys = parse_evaluator_response(
        r#"{"answ\u0065r":"yes","evid\u0065nce":"escaped keys","scop\u0065":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(escaped_keys.answer, "yes");
    assert_eq!(escaped_keys.evidence, "escaped keys");
    let canonicalized = parse_evaluator_response(
        r#"{"answer":"no","evidence":"code says no","scope":["src/check.rs","src"]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(canonicalized.answer, "no");
    assert_eq!(canonicalized.scope, vec!["src"]);
    assert!(parse_evaluator_response(
        r#"I checked the files first. {"answer":"yes","evidence":"README.md has evidence","scope":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "ANSWER: yes\nEVIDENCE:\nok\nSCOPE: [\".\"]",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes","evidence":"README.md has evidence","scope":["."]} trailing prose"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes\nno","evidence":"bad","scope":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes\u2028no","evidence":"bad","scope":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"a","evidence":"option a applies","scope":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "a"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"Rust","evidence":"Cargo.toml shows a Rust crate","scope":["Cargo.toml"]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "Rust"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"maybe","evidence":"question asks for this exact answer","scope":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "maybe"
    );
    assert!(parse_evaluator_response(
        r#"{"answer":"yes","evidence":"ok","scope":["."]} trailing prose"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "yes",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
}
