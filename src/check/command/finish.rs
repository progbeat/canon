use crate::check::command::output::{
    render_check_agent_messages, summary_outcome_counts, write_stdout_record,
};
use crate::check::core::types::{CheckRecord, CheckRunReport, SelectedExpectation};
use crate::check::interrogation::write_check_lifecycle_finish_event;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::gate::{
    gate_cached_result_for_tree, gate_regression_count_with_config, GateCacheResult,
    GateComparisonTree,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::history::HistoryCache;
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
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for passing in report_passing_expectations(report, agent) {
        match staged_visible_tree_matches_head(root, agent, &passing.scope, visible_tree_oid_cache)?
        {
            Some(true) | None => continue,
            Some(false) => {}
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
) -> Result<Option<bool>, String> {
    let Some(staged) = visible_tree_oid_cache.visible_tree_oid_for_reuse(
        root,
        &TreeSource::Staged,
        agent,
        scope,
    )?
    else {
        return Ok(None);
    };
    let head = visible_tree_oid_cache.gate_head_tree_fingerprint(root, agent, scope)?;
    Ok(Some(head.as_deref() == Some(staged.as_str())))
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
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Vec<String>, String> {
    let agent = &config.agent;
    let num_fixes = staged_passes_failed_at_head_count_with_cache(
        root,
        agent,
        report,
        history_cache,
        visible_tree_oid_cache,
    )?;
    // This is the check-command spec's `num_regressions`. Reusing gate's
    // comparison keeps a same-tree commit instruction aligned with
    // expectation-related `canon gate` failures.
    let num_regressions =
        gate_regression_count_with_config(root, config, history_cache, visible_tree_oid_cache)?;
    let outcome_counts = summary_outcome_counts(report);
    let num_failed = outcome_counts.failed;
    let num_errors = outcome_counts.errors;
    Ok(render_check_agent_messages(
        num_failed,
        num_errors,
        num_fixes,
        num_regressions,
    ))
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
        agent: agent.clone(),
        cooldown: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::types::{CheckResult, CheckRunReport};
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
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_passes_failed_at_head_count_with_cache(
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
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

        let count = staged_passes_failed_at_head_count_with_cache(
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
            .visible_tree_oid(root, &TreeSource::Staged, agent, scope)
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
            }],
            cached: Vec::new(),
            evaluated: 1,
            skipped: 0,
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
