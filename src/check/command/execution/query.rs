use super::in_place::validate_in_place_query_expectation;
use crate::app::LazyAppServerRunner;
use crate::check::command::output::write_query_output;
use crate::check::command::{
    collect_check_token_usage, prepare_check_execution, print_token_usage_summary,
    PrepareCheckExecutionOptions,
};
use crate::check::core::errors::error_record_from_interrogation_error;
use crate::check::core::QueryResult;
use crate::check::interrogation::policy::initial_visible_scope_for_expectation;
use crate::check::interrogation::query::{
    query_human_review_reason, run_query_with_runner, QueryExpectationContext, QueryRequest,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    write_query_lifecycle_finish_event, write_query_lifecycle_start_event,
};
use crate::check::run::selection::parse_cooldown;
use crate::check::{expectation_identities, CheckRecord, CheckRunCaches, SelectedExpectation};
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use std::io;
use std::path::Path;

pub(crate) struct CheckQueryCommand<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) question: &'a str,
    pub(crate) query_scope: &'a [String],
    pub(crate) query_scope_provided: bool,
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) against_tree: Option<&'a TreeSource>,
    pub(crate) no_sandbox: bool,
    pub(crate) in_place: bool,
    pub(crate) diagnostic_log: Option<DiagnosticLogWriter>,
    pub(crate) check_caches: &'a mut CheckRunCaches,
}

pub(crate) fn run_check_query_command(command: CheckQueryCommand<'_>) -> Result<(), String> {
    let CheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        diagnostic_log,
        check_caches,
    } = command;
    let mut diagnostic_log = diagnostic_log;
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_start_event(writer).map_err(|err| err.to_string())?;
    }
    let result = run_started_check_query_command(StartedCheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        diagnostic_log: diagnostic_log.as_mut(),
        check_caches,
    });
    let finish_error = result.as_ref().err().map(String::as_str);
    if let Some(writer) = diagnostic_log.as_mut() {
        write_query_lifecycle_finish_event(writer, finish_error).map_err(|err| err.to_string())?;
    }
    result
}

struct StartedCheckQueryCommand<'a, 'b> {
    root: &'a Path,
    config: &'a CheckConfig,
    question: &'a str,
    query_scope: &'a [String],
    query_scope_provided: bool,
    tree_source: Option<&'a TreeSource>,
    against_tree: Option<&'a TreeSource>,
    no_sandbox: bool,
    in_place: bool,
    diagnostic_log: Option<&'b mut DiagnosticLogWriter>,
    check_caches: &'a mut CheckRunCaches,
}

fn run_started_check_query_command(
    command: StartedCheckQueryCommand<'_, '_>,
) -> Result<(), String> {
    let StartedCheckQueryCommand {
        root,
        config,
        question,
        query_scope,
        query_scope_provided,
        tree_source,
        against_tree,
        no_sandbox,
        in_place,
        mut diagnostic_log,
        check_caches,
    } = command;
    let mut enforced_scope = query_enforced_scope(query_scope)?;
    let mut in_place_runner;
    let (runtime, runner): (CheckRuntime<'_>, &mut LazyAppServerRunner) = if in_place {
        in_place_runner = LazyAppServerRunner::new(
            root,
            crate::check::config::validation::check_config_loads_plugins(config),
            &config.agent,
            no_sandbox,
        )?;
        (
            CheckRuntime::in_place(root, config, no_sandbox),
            &mut in_place_runner,
        )
    } else {
        let tree_source = tree_source.ok_or_else(|| "missing query tree source".to_string())?;
        let against_tree = against_tree.ok_or_else(|| "missing query against tree".to_string())?;
        let mut execution = prepare_check_execution(
            root,
            config,
            PrepareCheckExecutionOptions {
                tree_source,
                against_tree,
                no_sandbox,
            },
            &mut check_caches.visible_tree_oid,
        )?;
        let runtime = CheckRuntime::materialized(
            root,
            &execution.staged_view,
            &execution.tree_source,
            execution.tree_context.clone(),
            config,
            no_sandbox,
        );
        let runner = &mut execution.runner;
        return run_prepared_query(
            root,
            runtime,
            runner,
            config,
            question,
            query_scope_provided,
            &mut enforced_scope,
            &mut diagnostic_log,
            check_caches,
        );
    };
    run_prepared_query(
        root,
        runtime,
        runner,
        config,
        question,
        query_scope_provided,
        &mut enforced_scope,
        &mut diagnostic_log,
        check_caches,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_prepared_query(
    root: &Path,
    runtime: CheckRuntime<'_>,
    runner: &mut LazyAppServerRunner,
    config: &CheckConfig,
    question: &str,
    query_scope_provided: bool,
    enforced_scope: &mut Vec<String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
) -> Result<(), String> {
    let query_expectation = query_expectation_context(config, question)?;
    if let Some(expectation) = query_expectation.as_ref() {
        if runtime.is_in_place() {
            validate_in_place_query_expectation(expectation)?;
            *enforced_scope = runtime
                .fresh_scope_without_persistent_q_scope()
                .expect("in-place query has no persistent q-scope");
        } else if !query_scope_provided {
            *enforced_scope = initial_visible_scope_for_expectation(
                root,
                runtime
                    .tree_source()
                    .ok_or_else(|| "missing query tree source".to_string())?,
                expectation,
                &mut check_caches.xpec_state,
                &mut check_caches.visible_tree_oid,
            )?;
        }
    }
    // Explicit `-s` changes the scope used for this query, but a matched
    // q/a-only expectation still records the result produced under that scope.
    // It must not seed future check runs because the scope was chosen by the
    // caller rather than accepted by interrogation policy.
    let persist_expectation_record = query_expectation.is_some();
    let seed_stored_q_scope = !query_scope_provided;
    // Query mode always asks the evaluator; it does not reuse cached results.
    // A matched q/a expectation reads only the last pass so query prompts can
    // preserve checkpoint context for that expectation.
    let query_last_pass = if runtime.is_in_place() {
        None
    } else {
        query_expectation
            .as_ref()
            .map(|expectation| check_caches.xpec_state.read_last_pass(root, expectation))
            .transpose()?
            .flatten()
    };
    let expectation = query_expectation
        .as_ref()
        .map(|expectation| QueryExpectationContext {
            expectation,
            last_pass: query_last_pass.as_ref(),
        });
    let mut interrogation_run_state =
        InterrogationRunState::new(runtime.no_sandbox() || runtime.is_in_place())?;
    let result = run_query_with_runner(
        &runtime,
        QueryRequest {
            question,
            enforced_scope,
            expectation,
        },
        runner,
        diagnostic_log.as_deref_mut(),
        &mut interrogation_run_state,
    );
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            persist_query_error_result(
                root,
                &runtime,
                runtime.checked_tree_oid(),
                check_caches,
                persist_expectation_record && !runtime.is_in_place(),
                seed_stored_q_scope,
                query_expectation.as_ref(),
                enforced_scope,
                &err,
            )?;
            print_query_token_usage(runner)?;
            return Err(err);
        }
    };
    persist_query_result(
        root,
        runtime.checked_tree_oid(),
        check_caches,
        persist_expectation_record && !runtime.is_in_place(),
        seed_stored_q_scope,
        &result,
    )?;
    if let Some(reason) = query_human_review_reason(&result) {
        print_query_token_usage(runner)?;
        return Err(format!("query requires human review: {}", reason));
    }
    write_successful_query_output(&result, runner)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_query_error_result(
    root: &Path,
    runtime: &CheckRuntime<'_>,
    checked_tree_oid: &str,
    check_caches: &mut CheckRunCaches,
    should_persist: bool,
    seed_stored_q_scope: bool,
    expectation: Option<&SelectedExpectation>,
    scope: &[String],
    error: &str,
) -> Result<(), String> {
    if !should_persist {
        return Ok(());
    }
    let Some(expectation) = expectation else {
        return Ok(());
    };
    let record = error_record_from_interrogation_error(
        runtime,
        &expectation.agent,
        expectation,
        scope,
        error,
        &mut check_caches.visible_tree_oid,
    )?;
    write_query_last_result(
        root,
        checked_tree_oid,
        check_caches,
        expectation,
        &record,
        seed_stored_q_scope,
    )
}

fn persist_query_result(
    root: &Path,
    checked_tree_oid: &str,
    check_caches: &mut CheckRunCaches,
    should_persist: bool,
    seed_stored_q_scope: bool,
    result: &QueryResult,
) -> Result<(), String> {
    if !should_persist {
        return Ok(());
    }
    let Some(record) = result.record.as_ref() else {
        return Ok(());
    };
    write_query_last_result(
        root,
        checked_tree_oid,
        check_caches,
        &record.expectation,
        &record.record,
        seed_stored_q_scope,
    )
}

fn write_query_last_result(
    root: &Path,
    checked_tree_oid: &str,
    check_caches: &mut CheckRunCaches,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    seed_stored_q_scope: bool,
) -> Result<(), String> {
    let result = if seed_stored_q_scope {
        check_caches.xpec_state.write_last_result_for_record(
            root,
            checked_tree_oid,
            expectation,
            record,
        )
    } else {
        check_caches
            .xpec_state
            .write_last_result_for_record_without_stored_q_scope_seed(
                root,
                checked_tree_oid,
                expectation,
                record,
            )
    };
    result.map(|_| ())
}

fn write_successful_query_output(
    result: &QueryResult,
    runner: &mut LazyAppServerRunner,
) -> Result<(), String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_query_output(&mut stdout, &result.answer)?;
    print_query_token_usage(runner)
}

fn print_query_token_usage(runner: &mut LazyAppServerRunner) -> Result<(), String> {
    let usage = collect_check_token_usage(runner);
    print_token_usage_summary(usage)
}

fn query_enforced_scope(query_scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope(query_scope).map_err(|err| format!("--scope: {}", err))
}

fn query_expectation_context(
    config: &CheckConfig,
    question: &str,
) -> Result<Option<SelectedExpectation>, String> {
    let identities = expectation_identities(config)?;
    let matches = config
        .expectations
        .iter()
        .enumerate()
        .filter_map(|(index, expectation)| {
            (expectation.question_answer_only && expectation.q == question).then_some(index)
        })
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return Ok(None);
    };
    let identity = identities
        .get(*index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    let expectation = config
        .expectations
        .get(*index)
        .ok_or_else(|| "expectation identity count mismatch".to_string())?;
    let cooldown = expectation
        .cooldown
        .as_ref()
        .map(parse_cooldown)
        .transpose()?;
    // This is not check-run selection. It only recovers the q/a-only
    // expectation context needed for plain `canon check -q <q>` to use the
    // same prompt and state inputs as `canon check <ID>`.
    Ok(Some(SelectedExpectation {
        number: *index + 1,
        id: identity.id.clone(),
        display_id: identity.display_id.clone(),
        question: expectation.q.clone(),
        expected_answer: expectation.a.clone(),
        instructions: expectation.instructions.clone(),
        diff_from: expectation.diff_from.clone(),
        target: expectation.target.clone(),
        question_answer_only: expectation.question_answer_only,
        agent: expectation.agent.clone(),
        cooldown,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{
        CheckRecord, CheckResult, ParsedAnswer, QueryExpectationRecord, INTERNAL_ERROR_UNPARSABLE,
    };
    use crate::config_types::{AgentConfig, Expectation};
    use crate::hash::full_scope;
    use crate::xpec_state::LastResultStatus;

    #[test]
    fn query_expectation_context_matches_unique_qa_only_question() {
        let config = two_expectation_config();

        let selected = query_expectation_context(&config, "Does beta pass?")
            .unwrap()
            .unwrap();

        assert_eq!(selected.question, "Does beta pass?");
        assert!(selected.question_answer_only);
    }

    #[test]
    fn query_expectation_context_ignores_non_qa_only_matches() {
        let mut config = two_expectation_config();
        config.expectations[0].question_answer_only = false;

        let selected = query_expectation_context(&config, "Does alpha pass?").unwrap();

        assert!(selected.is_none());
    }

    #[test]
    fn query_expectation_context_preserves_cooldown() {
        let mut config = two_expectation_config();
        config.expectations[0].cooldown = Some(crate::config_types::CooldownConfig::Compact(
            "7d".to_string(),
        ));

        let selected = query_expectation_context(&config, "Does alpha pass?")
            .unwrap()
            .unwrap();

        let cooldown = selected.cooldown.unwrap();
        assert_eq!(cooldown.pass_seconds, Some(7 * 24 * 60 * 60));
        assert_eq!(cooldown.fail_seconds, None);
    }

    #[test]
    fn persist_query_result_writes_error_record_for_matched_query() {
        let root = temp_query_root("last-error");
        let config = two_expectation_config();
        let expectation = query_expectation_context(&config, "Does alpha pass?")
            .unwrap()
            .unwrap();
        let answer = ParsedAnswer::error(
            INTERNAL_ERROR_UNPARSABLE.to_string(),
            "technical failure".to_string(),
        );
        let record = CheckRecord {
            timestamp: crate::time::format_record_timestamp(1),
            number: expectation.number,
            result: CheckResult::Fail,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
            error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
            evidence: "technical failure".to_string(),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: "visible-tree".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
        };
        let result = QueryResult {
            answer,
            record: Some(QueryExpectationRecord {
                expectation: expectation.clone(),
                record,
            }),
        };
        let mut caches = CheckRunCaches::new();

        persist_query_result(&root, "checked-tree", &mut caches, true, true, &result).unwrap();

        let last_error = caches
            .xpec_state
            .read_last_error(&root, &expectation)
            .unwrap()
            .unwrap();
        assert_eq!(last_error.status, LastResultStatus::Error);
        assert_eq!(
            last_error
                .response
                .get("error")
                .and_then(serde_json::Value::as_str),
            Some(INTERNAL_ERROR_UNPARSABLE)
        );
        assert!(last_error.checked_tree_oid.is_none());
        assert!(last_error.visible_tree_oid.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn query_expectation_context_does_not_choose_ambiguous_question() {
        let mut config = two_expectation_config();
        config.expectations.push(expectation("Does alpha pass?"));
        config.expectations[2].a = "no".to_string();

        let selected = query_expectation_context(&config, "Does alpha pass?").unwrap();

        assert!(selected.is_none());
    }

    fn two_expectation_config() -> CheckConfig {
        CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: AgentConfig::implementation_default(),
            expectations: vec![
                expectation("Does alpha pass?"),
                expectation("Does beta pass?"),
            ],
        }
    }

    fn expectation(question: &str) -> Expectation {
        Expectation {
            q: question.to_string(),
            a: "yes".to_string(),
            instructions: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: true,
            agent: AgentConfig::implementation_default(),
            cooldown: None,
        }
    }

    fn temp_query_root(label: &str) -> std::path::PathBuf {
        let temp_dir = std::env::temp_dir().canonicalize().unwrap();
        let root = temp_dir.join(format!("canon-query-test-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }
}
