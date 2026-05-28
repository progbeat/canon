use super::*;

#[test]
fn parser_handles_json_answer_and_free_form_evidence() {
    let parsed = parse_evaluator_response(
        r#"{"answer":"yes","evidence":"line: one\nqScopeSuggestion: this is evidence\nanswer label inside evidence","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(parsed.answer, "yes");
    assert_eq!(
        parsed.evidence,
        "line: one\nqScopeSuggestion: this is evidence\nanswer label inside evidence"
    );
    assert_eq!(parsed.q_scope_suggestion, Some(vec![".".to_string()]));
    let escaped_keys = parse_evaluator_response(
        r#"{"answ\u0065r":"yes","evid\u0065nce":"escaped keys","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(escaped_keys.answer, "yes");
    assert_eq!(escaped_keys.evidence, "escaped keys");
    let canonicalized = parse_evaluator_response(
        r#"{"answer":"no","evidence":"code says no","qScopeSuggestion":["src/check.rs","src"]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(canonicalized.answer, "no");
    assert_eq!(
        canonicalized.q_scope_suggestion,
        Some(vec!["src/check.rs".to_string(), "src".to_string()])
    );
    assert!(parse_evaluator_response(
        r#"I checked the files first. {"answer":"yes","evidence":"README.md has evidence","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "answer: yes\nevidence:\nok\nqScopeSuggestion: [\".\"]",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes","evidence":"README.md has evidence","qScopeSuggestion":["."]} trailing prose"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes\nno","evidence":"bad","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes\u2028no","evidence":"bad","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    let empty_answer = parse_evaluator_response(
        r#"{"answer":"","evidence":"blank answer","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap_err();
    assert_eq!(
        empty_answer,
        "answer must be a non-empty single-line string"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"a","evidence":"option a applies","qScopeSuggestion":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "a"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"Rust","evidence":"Cargo.toml shows a Rust crate","qScopeSuggestion":["Cargo.toml"]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "Rust"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"maybe","evidence":"question asks for this exact answer","qScopeSuggestion":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .answer,
        "maybe"
    );
    let insufficient = parse_evaluator_response(
        r#"{"error":"insufficient-evidence","evidence":"Need more files."}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(
        insufficient.error.as_deref(),
        Some(ERROR_INSUFFICIENT_EVIDENCE)
    );
    assert_eq!(insufficient.evidence, "Need more files.");
    assert_eq!(
        parse_evaluator_response(
            r#"{"error":"invalid-question","evidence":"Question is invalid."}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .error
        .as_deref(),
        Some(ERROR_INVALID_QUESTION)
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"error":"unparsable","evidence":"Cannot form JSON."}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap()
        .error
        .as_deref(),
        Some(ERROR_UNPARSABLE)
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"answer":"yes","error":"invalid-question","evidence":"bad"}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap_err(),
        "evaluator response must contain exactly one of answer or error"
    );
    assert_eq!(
        parse_evaluator_response(
            r#"{"evidence":"missing one-of field"}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap_err(),
        "evaluator response must contain exactly one of answer or error"
    );
    let evidence_with_backticks = parse_evaluator_response(
        r#"{"answer":"yes","evidence":"`src/check.rs` handles restricted error retry","qScopeSuggestion":["src/check.rs"]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(
        evidence_with_backticks.evidence,
        "`src/check.rs` handles restricted error retry"
    );
    let whitespace_evidence = parse_evaluator_response(
        r#"{"answer":"yes","evidence":"  \t ","qScopeSuggestion":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap_err();
    assert_eq!(whitespace_evidence, "evidence must be a non-empty string");
    assert!(parse_evaluator_response(
        r#"{"answer":"yes","evidence":"ok","qScopeSuggestion":["."]} trailing prose"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "yes",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
}
