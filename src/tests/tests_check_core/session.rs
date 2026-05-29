use crate::check::run_check_with_runner;
use crate::check_types::CheckRecord;
use crate::logging::DiagnosticLogWriter;
use crate::tests::{
    answer, check_config_yaml, check_options, enable_diagnostic_logs, git_project,
    parse_check_config, FakeRunner, TestDir,
};
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use std::fs;

#[test]
fn check_runner_hides_expected_answers_and_reuses_session() {
    let root = git_project("check-runner");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();
    assert!(records.records.iter().all(CheckRecord::passed));
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(log
        .lines()
        .any(|line| line.contains(r#""event":"thread.reuse""#)
            && line.contains(r#""baseInstructions":"#)
            && line.contains(r#""developerInstructions":"#)));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_roots, vec![root.clone()]);
    assert_eq!(
        runner.start_ignores,
        vec![vec![
            ".canon".to_string(),
            ".canon/**".to_string(),
            ".git/canon".to_string(),
            ".git/canon/**".to_string(),
            ".git/canon/logs".to_string(),
            ".git/canon/logs/**".to_string(),
            "target/**".to_string()
        ]]
    );
    assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
    assert_eq!(runner.start_models, vec![Some("gpt-5.4-mini".to_string())]);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
    assert_eq!(
        runner.ask_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.4-mini".to_string())
        ]
    );
    assert_eq!(
        runner.ask_thinking,
        vec!["medium".to_string(), "medium".to_string()]
    );
    assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
    assert!(runner
        .prompts
        .iter()
        .all(|prompt| !prompt.contains("Response format:")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_starts_sessions_from_snapshot_root() {
    let root = git_project("check-runner-real-root");
    let snapshot = TestDir::new("check-runner-snapshot-root");
    let snapshot_root = snapshot.path();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("yes", "README.md says enough", &["."])]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let records = run_check_with_runner(
        &root,
        &snapshot_root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.start_roots, vec![snapshot_root]);
    assert_ne!(runner.start_roots, vec![root.clone()]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_starts_new_thread_for_narrowed_visible_tree() {
    let root = git_project("check-runner-narrowed-thread");
    let config = parse_check_config(
        r#"
version: 1
presets:
  default:
    models:
      - gpt-5.4-mini
    thinking: medium
    instructions: Answer from files only.
    ignore: []
    plugins: []
expectations:
  - q: "Does src/main.rs answer yes?"
    a: "yes"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "src/main.rs says yes", &["src/main.rs"]),
        &answer("yes", "src/main.rs says yes", &["src/main.rs"]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 2);
    assert_eq!(
        runner.sessions,
        vec!["session-1".to_string(), "session-2".to_string()]
    );
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_retires_oversized_scope_thread_after_completed_expectation() {
    let root = git_project("check-runner-retire-oversized-thread");
    let config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
  thinking: medium
  instructions: Answer from files only.
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "no"
  - q: "Third?"
    a: "yes"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1", "2", "3"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "first answer", &["."]),
        &answer("no", "second answer", &["."]),
        &answer("yes", "third answer", &["."]),
    ]);
    runner
        .turn_usages
        .push_back(Some(turn_usage("session-1", "turn-1", 1_000, 1_000)));
    runner
        .turn_usages
        .push_back(Some(turn_usage("session-1", "turn-2", 49_000, 1_001)));
    runner
        .turn_usages
        .push_back(Some(turn_usage("session-2", "turn-3", 1_000, 1_000)));

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 2);
    assert_eq!(
        runner.sessions,
        vec![
            "session-1".to_string(),
            "session-1".to_string(),
            "session-2".to_string()
        ]
    );
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec![".".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_applies_thinking_per_turn_when_reusing_scope_thread() {
    let root = git_project("check-thinking-turn");
    let config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
  thinking: low
  instructions: Answer from files only.
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "yes"
    thinking: high
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("yes", "README.md says enough", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_thinking, vec!["low".to_string()]);
    assert_eq!(
        runner.ask_thinking,
        vec!["low".to_string(), "high".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

fn turn_usage(
    thread_id: &str,
    turn_id: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> EvaluatorTurnUsage {
    EvaluatorTurnUsage {
        thread_id: thread_id.to_string(),
        turn_id: turn_id.to_string(),
        usage: TokenUsage {
            total_tokens: input_tokens + output_tokens,
            input_tokens,
            cached_input_tokens: 0,
            output_tokens,
            reasoning_output_tokens: 0,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: Vec::new(),
    }
}
