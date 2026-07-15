use crate::check::command::output::{render_check_agent_messages, write_stdout_record};
use crate::check::core::{for_each_unique_report_record, CheckRunReport};
use crate::check::interrogation::write_check_lifecycle_finish_event;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use std::collections::BTreeSet;
use std::io::Write;

// This module is deliberately not the public check-output renderer. The
// per-expectation stdout records and summary line live in `command::output`,
// token usage stderr output lives in `command::reporting`, and
// `command::execution`
// orchestrates their order before calling `finish_check_report`. This module
// owns only the post-summary agent message plus finish logging. Success and
// error reports share finish logging and the post-summary message to the agent
// when allowed by the command form. The optional error only changes the finish
// log payload and final command result.
pub(crate) struct CheckReportFinishContext<'b> {
    pub(crate) diagnostic_log: Option<&'b mut crate::logs::DiagnosticLogWriter>,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) check_caches: &'b mut CheckRunCaches,
    pub(crate) write_agent_message: bool,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_>,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), CommandError> {
    // No eligible public output piece is pending here: per-expectation output
    // and the public trailer have already been rendered, written, and flushed
    // by their own writers. This post-trailer step cannot delay stdout/stderr
    // that was eligible earlier; it computes only the agent message and finish
    // lifecycle log.
    let mut post_finish_error = None;
    let mut finish_error = error.map(str::to_string);
    if context.write_agent_message {
        if let Err(err) =
            write_check_agent_message(report, context.result_output, context.check_caches)
        {
            finish_error.get_or_insert_with(|| err.to_string());
            post_finish_error.get_or_insert(err);
        }
    }
    if let Some(diagnostic_log) = context.diagnostic_log {
        write_check_lifecycle_finish_event(diagnostic_log, false, finish_error.as_deref())?;
    }
    if let Some(err) = post_finish_error {
        return Err(err);
    }
    Ok(())
}

fn write_check_agent_message(
    report: &CheckRunReport,
    output: &mut dyn Write,
    caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    let messages = check_agent_messages(report, &caches.run_start_pass_ids);
    for message in messages {
        let mut line = message;
        line.push('\n');
        write_stdout_record(output, line.as_bytes(), "check agent message")?;
    }
    Ok(())
}

pub(crate) fn check_agent_messages(
    report: &CheckRunReport,
    run_start_pass_ids: &BTreeSet<String>,
) -> Vec<String> {
    let num_new_passes = current_passes_without_prior_pass_count(report, run_start_pass_ids);
    let num_regressions = current_failures_with_prior_pass_count(report, run_start_pass_ids);
    let issue_ids = report_issue_display_ids(report);
    render_check_agent_messages(&issue_ids, num_new_passes, num_regressions, report.skipped)
}

fn report_issue_display_ids(report: &CheckRunReport) -> Vec<String> {
    let mut issue_ids = Vec::new();
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        if !record.passed() {
            issue_ids.push(record.display_id.clone());
        }
    });
    issue_ids
}

fn current_passes_without_prior_pass_count(
    report: &CheckRunReport,
    run_start_pass_ids: &BTreeSet<String>,
) -> usize {
    let mut count = 0usize;
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        if record.passed() && !run_start_pass_ids.contains(&record.id) {
            count += 1;
        }
    });
    count
}

fn current_failures_with_prior_pass_count(
    report: &CheckRunReport,
    run_start_pass_ids: &BTreeSet<String>,
) -> usize {
    let mut count = 0usize;
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        if !record.passed() && run_start_pass_ids.contains(&record.id) {
            count += 1;
        }
    });
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckRecord, CheckResult, CheckRunReport, ResolvedExpectation};
    use crate::config_types::{AgentConfig, CheckConfig, Expectation};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::hash::full_scope;
    use crate::time::format_record_timestamp;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: 8
    fn new_pass_emits_commit_message() {
        let root = git_project("new-pass-emits-commit-message");
        let agent = AgentConfig::default();
        let config = test_config(&agent);
        let expectation = test_expectation_from_config(&config);
        let scope = full_scope();
        let report = passing_report_for_staged_scope(&root, &expectation, &scope);

        let messages = check_agent_messages(&report, &BTreeSet::new());

        assert!(messages
            .iter()
            .any(|message| message.contains("Commit the staged changes")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("Fix the issues")));
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 8
    fn prior_pass_is_not_a_new_pass() {
        let root = git_project("prior-pass-is-not-a-new-pass");
        let agent = AgentConfig::default();
        let config = test_config(&agent);
        let expectation = test_expectation_from_config(&config);
        let scope = full_scope();
        let report = passing_report_for_staged_scope(&root, &expectation, &scope);
        let run_start_pass_ids = BTreeSet::from([expectation.id.clone()]);

        let messages = check_agent_messages(&report, &run_start_pass_ids);

        assert!(messages
            .iter()
            .any(|message| message.contains("All checks passed")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("Commit the staged changes")));
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 8
    fn prior_pass_regression_agent_message_repairs_instead_of_commits() {
        let root = git_project("prior-pass-regression-agent-message");
        let agent = AgentConfig::default();
        let config = test_config(&agent);
        let expectation = test_expectation_from_config(&config);
        let scope = full_scope();
        let report = CheckRunReport {
            records: vec![staged_scope_record(&root, &expectation, &scope, "no")],
            cached: Vec::new(),
            skipped: 0,
        };
        let run_start_pass_ids = BTreeSet::from([expectation.id.clone()]);

        let messages = check_agent_messages(&report, &run_start_pass_ids);

        assert!(messages
            .iter()
            .any(|message| message.contains("Fix the issues")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("Commit the staged changes")));
        let _ = fs::remove_dir_all(root);
    }

    fn passing_report_for_staged_scope(
        root: &std::path::Path,
        expectation: &ResolvedExpectation,
        scope: &[String],
    ) -> CheckRunReport {
        CheckRunReport {
            records: vec![staged_scope_record(root, expectation, scope, "yes")],
            cached: Vec::new(),
            skipped: 0,
        }
    }

    fn staged_scope_record(
        root: &std::path::Path,
        expectation: &ResolvedExpectation,
        scope: &[String],
        observed: &str,
    ) -> CheckRecord {
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(root, &TreeSource::Staged, &expectation.agent, scope)
            .unwrap();
        CheckRecord {
            timestamp: format_record_timestamp(0),
            number: expectation.number,
            result: CheckResult::from_expected_answer(&expectation.expected_answer, observed),
            to: crate::config_types::ExpectationTo::Agent,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: observed.to_string(),
            error: None,
            evidence: "test evidence".to_string(),
            scope: scope.to_vec(),
            question_scope_suggestion: None,
            visible_tree_oid,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
        }
    }

    fn test_config(agent: &AgentConfig) -> CheckConfig {
        CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: vec![Expectation {
                to: crate::config_types::ExpectationTo::Agent,
                rank: 0,
                q: "Does it pass?".to_string(),
                a: "yes".to_string(),
                question_context: String::new(),
                diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
                diff_from_configured: false,
                target: None,
                question_answer_only: false,
                agent: agent.clone(),
                cooldown: None,
            }],
        }
    }

    fn test_expectation_from_config(config: &CheckConfig) -> ResolvedExpectation {
        let identities = crate::check::expectation_identities(config).unwrap();
        crate::check::select_expectations_with_identities(config, &identities, &[])
            .unwrap()
            .remove(0)
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "core.eol", "lf"]);
        git(&root, &["config", "user.name", "Canon Test"]);
        git(&root, &["config", "user.email", "canon-test@example.com"]);
        fs::write(root.join("README.md"), "hello\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "initial"]);
        root
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
