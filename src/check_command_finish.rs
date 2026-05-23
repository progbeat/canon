use crate::check::CheckRunCaches;
use crate::check_lazy_reset::apply_lazy_full_scope_reset;
use crate::check_output::{record_requires_human_review, write_stdout_line_record};
use crate::check_reporting::write_check_finish_event;
use crate::check_types::{CheckRecord, CheckRunReport, SelectedExpectation};
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::gate::{exact_gate_cache_result_for_tree, GateCacheResult, GateComparisonTree};
use crate::history::HistoryCache;
use crate::scope_hash::ScopeHashCache;
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
    pub(crate) diagnostic_log: &'b mut crate::logging::DiagnosticLogWriter,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) check_caches: &'b mut CheckRunCaches,
    pub(crate) write_agent_message: bool,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_, '_>,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), CommandError> {
    // No earlier public output piece is pending here: per-expectation output
    // and the public trailer have already been rendered, written, and flushed
    // by their own writers. This step computes only the remaining post-trailer
    // side effects: the agent message, lazy reset, and finish lifecycle log.
    if context.write_agent_message {
        write_check_agent_message(
            context.root,
            context.config,
            report,
            context.result_output,
            context.check_caches,
        )?;
    }
    apply_lazy_full_scope_reset(
        context.root,
        context.config,
        report.evaluated,
        &report.non_selected,
        context.diagnostic_log,
    )?;
    write_check_finish_event(context.diagnostic_log, false, error)?;
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
    let mut scope_hash_cache = ScopeHashCache::new();
    staged_passes_failed_at_head_count_with_cache(
        root,
        agent,
        report,
        &mut history_cache,
        &mut scope_hash_cache,
    )
}

fn staged_passes_failed_at_head_count_with_cache(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for record in report.records.iter().filter(|record| record.passed()) {
        let Some(expectation) = selected_expectation_from_record(record) else {
            continue;
        };
        match exact_gate_cache_result_for_tree(
            root,
            agent,
            &expectation,
            GateComparisonTree::Head,
            history_cache,
            scope_hash_cache,
        )? {
            GateCacheResult::Fail(_) => count += 1,
            GateCacheResult::Pass | GateCacheResult::Missing => {}
        }
    }
    Ok(count)
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
        &config.agent,
        report,
        &mut caches.history,
        &mut caches.scope_hash,
    )?;
    for message in messages {
        write_stdout_line_record(output, &message, "check agent message")?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn check_agent_message(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<String, String> {
    Ok(check_agent_messages(root, agent, report, history_cache, scope_hash_cache)?.join("\n"))
}

pub(crate) fn check_agent_messages(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Vec<String>, String> {
    let num_fixes = staged_pass_notice_count(root, agent, report, history_cache, scope_hash_cache)?;
    let num_regressions =
        staged_regressions_count(root, agent, report, history_cache, scope_hash_cache)?;
    let num_failed = report
        .records
        .iter()
        .filter(|record| !record.passed() && !record_requires_human_review(record))
        .count();
    let num_errors = report
        .records
        .iter()
        .filter(|record| !record.passed() && record_requires_human_review(record))
        .count();
    let num_non_ok = num_failed + num_errors;
    if num_regressions > 0 || (num_non_ok > 0 && num_fixes == 0) {
        return Ok(vec![FIX_ISSUES_MESSAGE.to_string()]);
    }
    if num_non_ok == 0 && num_fixes == 0 {
        return Ok(vec![ALL_CHECKS_PASSED_MESSAGE.to_string()]);
    }
    let mut messages = vec![pass_improvement_notice(num_fixes).expect("positive fix count")];
    if num_non_ok > 0 {
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    }
    Ok(messages)
}

fn staged_regressions_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for record in report.records.iter().filter(|record| !record.passed()) {
        let Some(expectation) = selected_expectation_from_record(record) else {
            continue;
        };
        if matches!(
            exact_gate_cache_result_for_tree(
                root,
                agent,
                &expectation,
                GateComparisonTree::Head,
                history_cache,
                scope_hash_cache,
            )?,
            GateCacheResult::Pass
        ) {
            count += 1;
        }
    }
    Ok(count)
}

pub(crate) fn staged_pass_notice_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    staged_passes_failed_at_head_count_with_cache(
        root,
        agent,
        report,
        history_cache,
        scope_hash_cache,
    )
}

fn selected_expectation_from_record(record: &CheckRecord) -> Option<SelectedExpectation> {
    Some(SelectedExpectation {
        number: record.number,
        id: record.id.clone(),
        display_id: record.display_id.clone(),
        q: record.prompt.clone()?,
        a: record.expected.clone()?,
        cooldown: None,
        thinking: None,
    })
}
