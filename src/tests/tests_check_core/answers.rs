use crate::check::run_check_with_runner;
use crate::check_output::record_requires_human_review;
use crate::history::read_history_records;
use crate::tests::{
    answer, check_config_yaml, check_options, git_project, parse_check_config, FakeRunner,
};
use std::fs;

#[test]
fn check_runner_fails_mismatch_and_treats_idk_as_exact_string() {
    let root = git_project("check-fails");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("idk", "not enough", &["."]),
        &answer("yes", "wrong", &["."]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert!(!records.records[1].passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_reviews_free_form_yes_no_answer_shape() {
    let root = git_project("check-yes-no-free-form-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "Yes: concrete bug",
        "The answer field did not follow the yes/no policy",
        &["."],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "Yes: concrete bug");
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_passes_free_form_exact_answer() {
    let root = git_project("check-free-form-answer");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "What is this project implemented in?"
    a: "Rust"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "Rust",
        "README.md is a staged fixture path for this exact-string answer test",
        &["README.md"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "Rust");
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_records_free_form_exact_mismatch_history() {
    let root = git_project("check-free-form-answer-mismatch");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "What is this project implemented in?"
    a: "Rust"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "Python",
        "The evaluator returned a parsed single-line exact-string answer",
        &["README.md"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "Python");
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_reviews_reserved_tokens_for_free_form_exact_answers() {
    let root = git_project("check-free-form-answer-reserved-token");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "What is this project implemented in?"
    a: "Rust"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "malformed",
        "The evaluator marked the question as malformed",
        &["."],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "malformed");
    assert!(record_requires_human_review(&records.records[0]));
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}
