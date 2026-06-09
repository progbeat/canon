use crate::check::command::output::{render_check_agent_messages, write_stdout_record};
use crate::check::core::{
    for_each_unique_report_record, CheckRecord, CheckRunReport, SelectedExpectation,
};
use crate::check::interrogation::write_check_lifecycle_finish_event;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::gate::{gate_cached_result_for_tree, GateCacheResult, GateComparisonTree};
use crate::git::VisibleTreeOidCache;
use crate::history::HistoryCache;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

// This module is deliberately not the public check-output renderer. The
// per-expectation stdout records and summary line live in `command::output`,
// token usage stderr output lives in `command::reporting`, and
// `command::execution`
// orchestrates their order before calling `finish_check_report`. This module
// owns only the post-summary agent message plus finish logging. Success and
// error reports share finish logging and the post-summary message to the agent
// when allowed by the command form. The optional error only changes the finish
// log payload and final command result.
pub(crate) struct CheckReportFinishContext<'a, 'b> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) diagnostic_log: &'b mut crate::logs::DiagnosticLogWriter,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) check_caches: &'b mut CheckRunCaches,
    pub(crate) write_agent_message: bool,
    pub(crate) checked_tree_matches_against_tree: bool,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_, '_>,
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
        if let Err(err) = write_check_agent_message(
            context.root,
            context.config,
            report,
            context.result_output,
            context.check_caches,
            context.checked_tree_matches_against_tree,
        ) {
            finish_error.get_or_insert_with(|| err.to_string());
            post_finish_error.get_or_insert(err);
        }
    }
    write_check_lifecycle_finish_event(context.diagnostic_log, false, finish_error.as_deref())?;
    if let Some(err) = post_finish_error {
        return Err(err);
    }
    Ok(())
}

fn staged_passes_failed_at_head_count_with_cache(
    root: &Path,
    checked_tree_matches_against_tree: bool,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    if checked_tree_matches_against_tree {
        return Ok(0);
    }
    let mut count = 0usize;
    for passing in report_passing_expectations(report, agent) {
        match gate_cached_result_for_tree(
            root,
            agent,
            &passing.expectation,
            GateComparisonTree::Head,
            history_cache,
            visible_tree_oid_cache,
        )? {
            GateCacheResult::Fail => count += 1,
            GateCacheResult::Missing => {
                if !head_visible_tree_matches_passing_record(
                    root,
                    agent,
                    &passing,
                    visible_tree_oid_cache,
                )? {
                    count += 1;
                }
            }
            GateCacheResult::Pass => {}
        }
    }
    Ok(count)
}

struct PassingExpectation {
    expectation: SelectedExpectation,
    scope: Vec<String>,
    visible_tree_oid: String,
}

fn head_visible_tree_matches_passing_record(
    root: &Path,
    agent: &AgentConfig,
    passing: &PassingExpectation,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<bool, String> {
    let head_visible_tree_oid =
        visible_tree_oid_cache.gate_head_tree_fingerprint(root, agent, &passing.scope)?;
    Ok(head_visible_tree_oid.as_deref() == Some(passing.visible_tree_oid.as_str()))
}

fn report_passing_expectations(
    report: &CheckRunReport,
    agent: &AgentConfig,
) -> Vec<PassingExpectation> {
    let mut expectations = Vec::new();
    for record in report.records.iter().filter(|record| record.passed()) {
        if let Some(expectation) = selected_expectation_from_record(record, agent) {
            expectations.push(PassingExpectation {
                expectation,
                scope: record.scope.clone(),
                visible_tree_oid: record.visible_tree_oid.clone(),
            });
        }
    }
    for cached in report.cached.iter().filter(|cached| cached.record.passed()) {
        expectations.push(PassingExpectation {
            expectation: cached.expectation.clone(),
            scope: cached.record.scope.clone(),
            visible_tree_oid: cached.record.visible_tree_oid.clone(),
        });
    }
    expectations
}

fn current_non_passes_with_prior_pass_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for expectation in report_non_passing_expectations(report, agent) {
        let records = history_cache.read_records(root, &expectation)?;
        if records.iter().any(CheckRecord::passed) {
            count += 1;
        }
    }
    Ok(count)
}

fn report_non_passing_expectations(
    report: &CheckRunReport,
    agent: &AgentConfig,
) -> Vec<SelectedExpectation> {
    let mut seen = BTreeSet::new();
    let mut expectations = Vec::new();
    for record in report.records.iter().filter(|record| !record.passed()) {
        if let Some(expectation) = selected_expectation_from_record(record, agent) {
            if seen.insert(expectation.id.clone()) {
                expectations.push(expectation);
            }
        }
    }
    for cached in report
        .cached
        .iter()
        .filter(|cached| !cached.record.passed())
    {
        if seen.insert(cached.expectation.id.clone()) {
            expectations.push(cached.expectation.clone());
        }
    }
    expectations
}

fn write_check_agent_message(
    root: &Path,
    config: &CheckConfig,
    report: &CheckRunReport,
    output: &mut dyn Write,
    caches: &mut CheckRunCaches,
    checked_tree_matches_against_tree: bool,
) -> Result<(), CommandError> {
    let messages = check_agent_messages(
        root,
        config,
        report,
        checked_tree_matches_against_tree,
        &mut caches.history,
        &mut caches.visible_tree_oid,
    )?;
    for message in messages {
        let mut line = message;
        line.push('\n');
        write_stdout_record(output, line.as_bytes(), "check agent message")?;
    }
    Ok(())
}

pub(crate) fn check_agent_messages(
    root: &Path,
    config: &CheckConfig,
    report: &CheckRunReport,
    checked_tree_matches_against_tree: bool,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Vec<String>, String> {
    let agent = &config.agent;
    let num_fixes = staged_passes_failed_at_head_count_with_cache(
        root,
        checked_tree_matches_against_tree,
        agent,
        report,
        history_cache,
        visible_tree_oid_cache,
    )?;
    let num_regressions =
        current_non_passes_with_prior_pass_count(root, agent, report, history_cache)?;
    let issue_ids = report_issue_display_ids(report);
    Ok(render_check_agent_messages(
        &issue_ids.failed,
        &issue_ids.errors,
        num_fixes,
        num_regressions,
    ))
}

struct IssueDisplayIds {
    failed: Vec<String>,
    errors: Vec<String>,
}

fn report_issue_display_ids(report: &CheckRunReport) -> IssueDisplayIds {
    let mut issue_ids = IssueDisplayIds {
        failed: Vec::new(),
        errors: Vec::new(),
    };
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        if record.passed() {
            return;
        }
        if record.requires_human_review() {
            issue_ids.errors.push(record.display_id.clone());
        } else {
            issue_ids.failed.push(record.display_id.clone());
        }
    });
    issue_ids
}

fn selected_expectation_from_record(
    record: &CheckRecord,
    agent: &AgentConfig,
) -> Option<SelectedExpectation> {
    Some(SelectedExpectation {
        number: record.number,
        id: record.id.clone(),
        display_id: record.display_id.clone(),
        question: record.question.clone()?,
        expected_answer: record.expected_answer.clone()?,
        question_answer_only: false,
        agent: agent.clone(),
        cooldown: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckResult, CheckRunReport};
    use crate::git::TreeSource;
    use crate::hash::full_scope;
    use crate::history::append_current_history_record_with_cache;
    use crate::time::format_record_timestamp;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn same_visible_tree_pass_with_missing_head_cache_is_not_an_improvement() {
        let root = git_project("same-visible-tree-pass-no-improvement");
        let agent = AgentConfig::default();
        let scope = full_scope();
        let report = passing_report_for_staged_scope(&root, &agent, &scope);
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_passes_failed_at_head_count_with_cache(
            &root,
            false,
            &agent,
            &report,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(count, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn changed_visible_tree_pass_with_missing_head_cache_is_an_improvement() {
        let root = git_project("changed-visible-tree-pass-improvement");
        fs::write(root.join("README.md"), "changed\n").unwrap();
        git(&root, &["add", "README.md"]);
        let agent = AgentConfig::default();
        let scope = full_scope();
        let report = passing_report_for_staged_scope(&root, &agent, &scope);
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_passes_failed_at_head_count_with_cache(
            &root,
            false,
            &agent,
            &report,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn matching_run_trees_ignore_late_staged_pass_improvement() {
        let root = git_project("matching-run-trees-late-staged-pass");
        let agent = AgentConfig::default();
        let scope = full_scope();
        fs::write(root.join("README.md"), "changed after preparation\n").unwrap();
        git(&root, &["add", "README.md"]);
        let report = passing_report_for_staged_scope(&root, &agent, &scope);
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_passes_failed_at_head_count_with_cache(
            &root,
            true,
            &agent,
            &report,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(count, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn current_non_pass_with_any_prior_pass_is_a_regression() {
        let root = git_project("current-non-pass-prior-pass-regression");
        let agent = AgentConfig::default();
        let scope = full_scope();
        let pass_record = staged_scope_record(&root, &agent, &scope, "yes");
        let expectation = selected_expectation_from_record(&pass_record, &agent).unwrap();
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        append_current_history_record_with_cache(
            &root,
            &TreeSource::Staged,
            &expectation,
            &pass_record,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();
        let fail_record = staged_scope_record(&root, &agent, &scope, "no");
        let report = CheckRunReport {
            records: vec![fail_record],
            cached: Vec::new(),
            evaluated: 1,
            skipped: 0,
        };

        let count =
            current_non_passes_with_prior_pass_count(&root, &agent, &report, &mut history_cache)
                .unwrap();

        assert_eq!(count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prior_pass_regression_agent_message_repairs_instead_of_commits() {
        let root = git_project("prior-pass-regression-agent-message");
        let agent = AgentConfig::default();
        let scope = full_scope();
        let pass_record = staged_scope_record(&root, &agent, &scope, "yes");
        let expectation = selected_expectation_from_record(&pass_record, &agent).unwrap();
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        append_current_history_record_with_cache(
            &root,
            &TreeSource::Staged,
            &expectation,
            &pass_record,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();
        let report = CheckRunReport {
            records: vec![staged_scope_record(&root, &agent, &scope, "no")],
            cached: Vec::new(),
            evaluated: 1,
            skipped: 0,
        };
        let config = CheckConfig {
            version: 1,
            presets: BTreeMap::new(),
            agent: agent.clone(),
            expectations: Vec::new(),
        };

        let messages = check_agent_messages(
            &root,
            &config,
            &report,
            false,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(
            messages,
            render_check_agent_messages(&["1".to_string()], &[], 0, 1)
        );
        assert!(messages.iter().any(|message| message.contains(
            "run `canon show not:1 [not:<ALREADY_IN_CONTEXT_EXPECTATION>]... -- <PATHSPEC>...`"
        )));
        let _ = fs::remove_dir_all(root);
    }

    fn passing_report_for_staged_scope(
        root: &std::path::Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> CheckRunReport {
        CheckRunReport {
            records: vec![staged_scope_record(root, agent, scope, "yes")],
            cached: Vec::new(),
            evaluated: 1,
            skipped: 0,
        }
    }

    fn staged_scope_record(
        root: &std::path::Path,
        agent: &AgentConfig,
        scope: &[String],
        observed: &str,
    ) -> CheckRecord {
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(root, &TreeSource::Staged, agent, scope)
            .unwrap();
        CheckRecord {
            timestamp: format_record_timestamp(0),
            number: 1,
            result: CheckResult::from_expected_answer("yes", observed),
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: None,
            evidence: "test evidence".to_string(),
            scope: scope.to_vec(),
            question_scope_suggestion: None,
            visible_tree_oid,
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
        }
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
