use super::order::order_by_latest_non_pass;
use crate::check::core::{CheckOptions, SelectedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) struct CacheFilteredCheckWork {
    // Expectations that still require evaluator work. Cached hits are excluded
    // from this queue and are not selected evaluations.
    pub(crate) to_evaluate: Vec<SelectedExpectation>,
    pub(crate) cached_hits: Vec<CachedExpectationHit>,
}

pub(crate) struct CachedExpectationHit {
    pub(crate) expectation: SelectedExpectation,
    pub(crate) hit: CheckCacheHit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedFailureMode {
    Continue,
    StopDefaultSelection,
}

pub(crate) struct CacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// Cache filtering receives command-independent `CheckOptions`; CLI/trailer
// policy stays in the command layer before and after this selection step.
pub(crate) fn select_expectations_after_cache(
    context: CacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
    cached_failure_mode: CachedFailureMode,
) -> Result<CacheFilteredCheckWork, String> {
    if options.selectors_provided {
        return Ok(CacheFilteredCheckWork {
            to_evaluate: options.selected.clone(),
            cached_hits: Vec::new(),
        });
    }

    let mut to_evaluate = Vec::new();
    let mut cached_hits = Vec::new();
    let mut cached_failure_seen = false;
    for expectation in options.selected.clone() {
        // Cached results come only from pass/fail same-tree or cooldown state;
        // last-error human-review records remain non-pass history for ordering,
        // but are not cache hits.
        match cached_result_for_expectation(
            context.root,
            context.source,
            &expectation.agent,
            &expectation,
            &mut *context.xpec_state,
            &mut *context.visible_tree_oid_cache,
            CachedResultLookup {
                now,
                include_same_tree: true,
                include_cooldown: !options.ignore_cooldown,
            },
        )? {
            Some(hit) => {
                cached_failure_seen |= !hit.record.passed();
                if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)?;
                }
                cached_hits.push(CachedExpectationHit { expectation, hit });
            }
            None => to_evaluate.push(expectation),
        }
    }
    if cached_failure_seen && cached_failure_mode == CachedFailureMode::StopDefaultSelection {
        // Selected Expectations default policy: when a no-selector run sees a
        // cached failure, the selected queue is empty until that cached failure
        // is fixed. This is before evaluation starts, so canon-check-order's
        // evaluated-expectation ordering and stop-after-non-pass rule is not
        // reached for the cleared expectations.
        to_evaluate.clear();
        cached_hits =
            order_by_latest_non_pass(context.root, cached_hits, context.xpec_state, |hit| {
                &hit.expectation
            })?;
    }
    Ok(CacheFilteredCheckWork {
        to_evaluate,
        cached_hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckRecord, CheckResult, Cooldown};
    use crate::config_types::{AgentConfig, ExpectationTarget};
    use crate::hash::full_scope;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn selector_runs_bypass_cached_results() {
        let root = git_project("selector-bypasses-cache");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
        git(&root, &["add", "src/lib.rs"]);

        let expectation = test_expectation();
        let source = TreeSource::Staged;
        let scope = full_scope();
        let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
        let visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &source, &expectation.agent, &scope)
            .unwrap();
        XpecStateCache::default()
            .write_last_result_for_record(
                &root,
                &checked_tree_oid,
                &expectation,
                &test_record(&expectation, &scope, "no", visible_tree_oid),
            )
            .unwrap();

        let default_work = cache_filtered_work(&root, &source, expectation.clone(), false);
        assert_eq!(default_work.cached_hits.len(), 1);
        assert!(default_work.to_evaluate.is_empty());

        let selector_work = cache_filtered_work(&root, &source, expectation, true);
        assert!(selector_work.cached_hits.is_empty());
        assert_eq!(selector_work.to_evaluate.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    fn cache_filtered_work(
        root: &Path,
        source: &TreeSource,
        expectation: SelectedExpectation,
        selectors_provided: bool,
    ) -> CacheFilteredCheckWork {
        let mut xpec_state = XpecStateCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let mut diagnostic_log = None;
        select_expectations_after_cache(
            CacheFilterContext {
                root,
                source,
                xpec_state: &mut xpec_state,
                visible_tree_oid_cache: &mut visible_tree_oid_cache,
                diagnostic_log: &mut diagnostic_log,
            },
            &CheckOptions {
                selected: vec![expectation],
                selectors_provided,
                keep_going: false,
                ignore_cooldown: false,
                break_after_tokens: None,
            },
            2,
            CachedFailureMode::Continue,
        )
        .unwrap()
    }

    fn test_expectation() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "abc123".to_string(),
            display_id: "a".to_string(),
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            instructions: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: Option::<ExpectationTarget>::None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: Option::<Cooldown>::None,
        }
    }

    fn test_record(
        expectation: &SelectedExpectation,
        scope: &[String],
        observed: &str,
        visible_tree_oid: String,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: crate::time::format_record_timestamp(1),
            number: expectation.number,
            result: CheckResult::from_expected_answer(&expectation.expected_answer, observed),
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: observed.to_string(),
            error: None,
            evidence: "evidence".to_string(),
            scope: scope.to_vec(),
            question_scope_suggestion: Some(scope.to_vec()),
            visible_tree_oid,
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
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
            .join(format!(
                "canon-cache-select-{}-{}-{}",
                name,
                process::id(),
                unique
            ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
