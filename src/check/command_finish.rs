use crate::check::lazy_reset::apply_lazy_full_scope_reset;
use crate::check::output::{summary_outcome_counts, write_stdout_line_record};
use crate::check::reporting::write_check_finish_event;
use crate::check::types::{CheckRecord, CheckRunReport, SelectedExpectation};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::gate::{
    gate_cached_result_for_tree, gate_regression_count_with_config, GateCacheResult,
    GateComparisonTree,
};
use crate::git::visible_tree_oid::VisibleTreeOidCache;
use crate::history::HistoryCache;
use std::io::Write;
use std::path::Path;

// This module is deliberately not the public check-output renderer. The
// per-expectation stdout records and summary line live in `check_output`, token
// usage stderr output lives in `check_reporting`, and `check_command`
// orchestrates their order before calling `finish_check_report`. This module
// owns only the post-summary agent message plus cleanup and finish logging.
const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed. Commit is allowed.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
const THEN_FIX_REMAINING_MESSAGE: &str =
    "▷ Then fix the remaining issues and run `canon check` again!";
const PASS_IMPROVEMENT_COMMIT_SUFFIX: &str = "Commit the staged changes NOW!";

// Success and error reports share cleanup, finish logging, and the post-summary
// message to the agent when allowed by the command form. The optional error
// only changes the finish log payload and final command result.
pub(crate) struct CheckReportFinishContext<'a, 'b> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) diagnostic_log: &'b mut crate::logs::DiagnosticLogWriter,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) check_caches: &'b mut CheckRunCaches,
    pub(crate) write_agent_message: bool,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_, '_>,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), CommandError> {
    // No eligible public output piece is pending here: per-expectation output
    // and the public trailer have already been rendered, written, and flushed
    // by their own writers. This post-trailer step cannot delay stdout/stderr
    // that was eligible earlier; it computes only the agent message, lazy
    // reset, and finish lifecycle log.
    let mut post_finish_error = None;
    let mut finish_error = error.map(str::to_string);
    if context.write_agent_message {
        if let Err(err) = write_check_agent_message(
            context.root,
            context.config,
            report,
            context.result_output,
            context.check_caches,
        ) {
            finish_error.get_or_insert_with(|| err.to_string());
            post_finish_error.get_or_insert(err);
        }
    }
    if let Err(err) = apply_lazy_full_scope_reset(
        context.root,
        context.config,
        report.evaluated,
        &report.cached,
        context.diagnostic_log,
    ) {
        finish_error.get_or_insert_with(|| err.clone());
        post_finish_error.get_or_insert_with(|| err.into());
    }
    write_check_finish_event(context.diagnostic_log, false, finish_error.as_deref())?;
    if let Some(err) = post_finish_error {
        return Err(err);
    }
    Ok(())
}

pub(crate) fn pass_improvement_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!(
            "▷ +1 pass compared to HEAD. {}",
            PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
        count => Some(format!(
            "▷ +{} passes compared to HEAD. {}",
            count, PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
    }
}

#[cfg(test)]
pub(crate) fn staged_passes_failed_at_head_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
) -> Result<usize, String> {
    let mut history_cache = HistoryCache::new();
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    staged_passes_failed_at_head_count_with_cache(
        root,
        agent,
        report,
        &mut history_cache,
        &mut visible_tree_oid_cache,
    )
}

fn staged_passes_failed_at_head_count_with_cache(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for passing in report_passing_expectations(report, agent) {
        if staged_visible_tree_matches_head(root, agent, &passing.scope, visible_tree_oid_cache)? {
            continue;
        }
        match gate_cached_result_for_tree(
            root,
            agent,
            &passing.expectation,
            GateComparisonTree::Head,
            history_cache,
            visible_tree_oid_cache,
        )? {
            GateCacheResult::Fail | GateCacheResult::Missing => count += 1,
            GateCacheResult::Pass => {}
        }
    }
    Ok(count)
}

struct PassingExpectation {
    expectation: SelectedExpectation,
    scope: Vec<String>,
}

fn staged_visible_tree_matches_head(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<bool, String> {
    let staged = visible_tree_oid_cache.staged_visible_tree_oid(root, agent, scope)?;
    let head = visible_tree_oid_cache.gate_head_tree_fingerprint(root, agent, scope)?;
    Ok(head.as_deref() == Some(staged.as_str()))
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
            });
        }
    }
    for cached in report.cached.iter().filter(|cached| cached.record.passed()) {
        expectations.push(PassingExpectation {
            expectation: cached.expectation.clone(),
            scope: cached.record.scope.clone(),
        });
    }
    expectations
}

fn write_check_agent_message(
    root: &Path,
    config: &CheckConfig,
    report: &CheckRunReport,
    output: &mut dyn Write,
    caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    let messages = check_agent_messages(
        root,
        config,
        report,
        &mut caches.history,
        &mut caches.visible_tree_oid,
    )?;
    for message in messages {
        write_stdout_line_record(output, &message, "check agent message")?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn check_agent_message(
    root: &Path,
    config: &CheckConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<String, String> {
    Ok(
        check_agent_messages(root, config, report, history_cache, visible_tree_oid_cache)?
            .join("\n"),
    )
}

pub(crate) fn check_agent_messages(
    root: &Path,
    config: &CheckConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Vec<String>, String> {
    let agent = &config.agent;
    let num_fixes =
        staged_pass_notice_count(root, agent, report, history_cache, visible_tree_oid_cache)?;
    // This is the check-command spec's `num_regressions`. Reusing gate's
    // comparison keeps a same-tree commit instruction aligned with
    // expectation-related `canon gate` failures.
    let num_regressions =
        gate_regression_count_with_config(root, config, history_cache, visible_tree_oid_cache)?;
    let outcome_counts = summary_outcome_counts(report);
    let num_failed = outcome_counts.failed;
    let num_errors = outcome_counts.errors;
    let num_non_ok = num_failed + num_errors;
    if num_regressions > 0 || (num_non_ok > 0 && num_fixes == 0) {
        return Ok(vec![FIX_ISSUES_MESSAGE.to_string()]);
    }
    if num_non_ok == 0 && num_fixes == 0 {
        return Ok(vec![ALL_CHECKS_PASSED_MESSAGE.to_string()]);
    }
    // The commit notice is reachable only after `num_regressions == 0`.
    // `num_regressions` is computed by `gate_regression_count_with_config`, so
    // `canon gate` has no expectation-related failure branch left for this
    // staged tree even if non-regressing non-OK records remain.
    let mut messages = vec![pass_improvement_notice(num_fixes).expect("positive fix count")];
    if num_non_ok > 0 {
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    }
    Ok(messages)
}

pub(crate) fn staged_pass_notice_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    staged_passes_failed_at_head_count_with_cache(
        root,
        agent,
        report,
        history_cache,
        visible_tree_oid_cache,
    )
}

fn selected_expectation_from_record(
    record: &CheckRecord,
    agent: &AgentConfig,
) -> Option<SelectedExpectation> {
    Some(SelectedExpectation {
        number: record.number,
        id: record.id.clone(),
        display_id: record.display_id.clone(),
        q: record.prompt.clone()?,
        a: record.expected.clone()?,
        prompt_scope: Vec::new(),
        agent: agent.clone(),
        cooldown: None,
        thinking: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::types::{CheckResult, CheckRunReport, NarrowingStats};
    use crate::hash::full_scope;
    use crate::time::format_record_timestamp;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn same_visible_tree_pass_is_not_a_head_improvement() {
        let root = git_project("same-visible-tree-pass-not-improvement");
        let agent = AgentConfig::default();
        let scope = full_scope();
        let report = passing_report_for_staged_scope(&root, &agent, &scope);
        let mut history_cache = HistoryCache::new();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_pass_notice_count(
            &root,
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
        let mut history_cache = HistoryCache::new();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_pass_notice_count(
            &root,
            &agent,
            &report,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(count, 1);
        let _ = fs::remove_dir_all(root);
    }

    fn passing_report_for_staged_scope(
        root: &std::path::Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> CheckRunReport {
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let visible_tree_oid = visible_tree_oid_cache
            .staged_visible_tree_oid(root, agent, scope)
            .unwrap();
        CheckRunReport {
            records: vec![CheckRecord {
                timestamp: format_record_timestamp(0),
                number: 1,
                result: CheckResult::Pass,
                prompt: Some("Does it pass?".to_string()),
                expected: Some("yes".to_string()),
                observed: "yes".to_string(),
                error: None,
                evidence: "test evidence".to_string(),
                scope: scope.to_vec(),
                suggested_q_scope: None,
                visible_tree_oid,
                id: "11111111111111111111".to_string(),
                display_id: "1".to_string(),
                cache_key: None,
            }],
            non_selected: Vec::new(),
            cached: Vec::new(),
            evaluated: 1,
            selected: 1,
            skipped: 0,
            silent: 0,
            narrowing: NarrowingStats::default(),
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
