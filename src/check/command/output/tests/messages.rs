use super::*;

#[test] // xpec: l
fn successful_query_output_matches_ask_mode_agent_contract() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(CapturedOutput {
        bytes: bytes.clone(),
    }));
    let report = start_query_report_output(output, "q");
    let answer = ParsedAnswer::answer(
        EvaluationAnswer::new("yes".to_string()),
        "test evidence".to_string(),
        Some(vec!["src/check".to_string()]),
    );
    let result = QueryResult {
        answer,
        diff_from: Some(":against-tree".to_string()),
        diff_from_tree_oid_abbrev: Some("abc1234".to_string()),
    };

    finish_query_output(report, &result, None).unwrap();

    assert_eq!(
        captured_string(&bytes),
        "q.\ndiff-from: abc1234 (:against-tree)\nobserved: yes\nevidence: test evidence\nq-scope-suggestion: [\"src/check\"]\n"
    );
}
