use super::order::order_by_latest_non_pass;
use crate::check::core::{CheckOptions, ResolvedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) enum GitBackedCacheFilteredCheckWork {
    EvaluationAllowed {
        // Expectations that cache policy sends to the evaluator.
        evaluation_queue: Vec<ResolvedExpectation>,
        cached_hits: Vec<CachedExpectationHit>,
    },
    DefaultSelectionEmptyWithCachedNonPassReports {
        // xpec: nT
        // A cached non-pass blocks default selection before evaluation starts:
        // uncached candidates remain pending instead of becoming selected
        // evaluator work, so `e5` has no uncached selected work to order here.
        cached_hits: Vec<CachedExpectationHit>,
    },
}

pub(crate) struct CachedExpectationHit {
    pub(crate) expectation: ResolvedExpectation,
    pub(crate) hit: CheckCacheHit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedNonPassPolicy {
    // Explicit selectors force uncached candidates into selected evaluator work.
    EvaluateUncachedCandidates,
    // Default mode with any cached non-pass has selected = empty; uncached
    // candidates are pending, not selected work ordered by e5.
    EmptySelectionLeavesUncachedPending,
}

pub(crate) struct GitBackedCacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// Git-backed cache filtering applies nT selection before evaluation starts:
// selector mode evaluates explicit selections, all-pass cached state selects
// uncached candidates, and any cached non-pass blocks default selection so
// uncached candidates remain pending.
//
// This layer only partitions work by cache availability and selectedness. The
// execution layer orders cached report hits and the remaining selected
// evaluator work together before running them. Cache selection reads existing
// xpec state and may emit bounded runtime-log events through the supplied
// writer, but it does not create a persistent state family of its own.
//
// This function receives command-independent `CheckOptions`; CLI/trailer policy
// stays in the command layer before and after this selection step.
pub(crate) fn select_git_backed_expectations_after_cache(
    context: GitBackedCacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
    cached_non_pass_policy: CachedNonPassPolicy,
) -> Result<GitBackedCacheFilteredCheckWork, String> {
    if options.selectors_provided {
        // Explicit expectation selectors are forced selections. Do not inspect
        // cached results here: a cache hit must not move an explicit candidate
        // out of the evaluator queue.
        return Ok(GitBackedCacheFilteredCheckWork::EvaluationAllowed {
            evaluation_queue: options.candidate_expectations.clone(),
            cached_hits: Vec::new(),
        });
    }

    let mut unselected_uncached_candidates = Vec::new();
    let mut cached_hits = Vec::new();
    let mut cached_non_pass_seen = false;
    for expectation in options.candidate_expectations.clone() {
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
                cached_non_pass_seen |= !hit.record.passed();
                if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)?;
                }
                cached_hits.push(CachedExpectationHit { expectation, hit });
            }
            None => unselected_uncached_candidates.push(expectation),
        }
    }
    let cached_non_pass_makes_default_selection_empty = cached_non_pass_seen
        && cached_non_pass_policy == CachedNonPassPolicy::EmptySelectionLeavesUncachedPending;
    if cached_non_pass_makes_default_selection_empty {
        // xpec: nT
        // Selected Expectations policy decides this before e5 ordering exists:
        // when a cached non-pass is present in default mode, the selected
        // evaluator set is empty and uncached candidates stay pending. The
        // cached hits returned here are report-only known results, not selected
        // expectations and not evaluator work ordered by e5.
        cached_hits =
            order_by_latest_non_pass(context.root, cached_hits, context.xpec_state, |hit| {
                &hit.expectation
            })?;
        return Ok(
            GitBackedCacheFilteredCheckWork::DefaultSelectionEmptyWithCachedNonPassReports {
                cached_hits,
            },
        );
    }
    Ok(GitBackedCacheFilteredCheckWork::EvaluationAllowed {
        evaluation_queue: unselected_uncached_candidates,
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

    #[test] // xpec: D,nT
    fn default_runs_reuse_cached_results() {
        let root = git_project("default-reuses-cache");
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

        let work = cache_filtered_work(&root, &source, expectation);
        match work {
            GitBackedCacheFilteredCheckWork::DefaultSelectionEmptyWithCachedNonPassReports {
                cached_hits,
            } => assert_eq!(cached_hits.len(), 1),
            GitBackedCacheFilteredCheckWork::EvaluationAllowed { .. } => {
                panic!("cached non-pass must block default selection")
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: nT
    fn selector_mode_forces_evaluation_despite_cached_results() {
        let root = git_project("selector-cache-continues");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
        git(&root, &["add", "src/lib.rs"]);

        let cached_expectation = test_expectation();
        let uncached_expectation = test_expectation_with_identity(2, "def456", "d");
        let source = TreeSource::Staged;
        let scope = full_scope();
        let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
        let visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &source, &cached_expectation.agent, &scope)
            .unwrap();
        XpecStateCache::default()
            .write_last_result_for_record(
                &root,
                &checked_tree_oid,
                &cached_expectation,
                &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work_with_mode(
            &root,
            &source,
            vec![cached_expectation, uncached_expectation],
            true,
            CachedNonPassPolicy::EvaluateUncachedCandidates,
        );
        match work {
            GitBackedCacheFilteredCheckWork::EvaluationAllowed {
                cached_hits,
                evaluation_queue,
            } => {
                assert!(cached_hits.is_empty());
                assert_eq!(evaluation_queue.len(), 2);
                assert_eq!(evaluation_queue[0].id, "abc123");
                assert_eq!(evaluation_queue[1].id, "def456");
            }
            GitBackedCacheFilteredCheckWork::DefaultSelectionEmptyWithCachedNonPassReports {
                ..
            } => {
                panic!("explicit selectors must force evaluation")
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: nT
    fn default_cached_non_pass_leaves_uncached_candidates_pending() {
        let root = git_project("default-cached-non-pass-leaves-uncached-pending");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
        git(&root, &["add", "src/lib.rs"]);

        let cached_expectation = test_expectation();
        let uncached_expectation = test_expectation_with_identity(2, "def456", "d");
        let source = TreeSource::Staged;
        let scope = full_scope();
        let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
        let visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &source, &cached_expectation.agent, &scope)
            .unwrap();
        XpecStateCache::default()
            .write_last_result_for_record(
                &root,
                &checked_tree_oid,
                &cached_expectation,
                &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work_with_mode(
            &root,
            &source,
            vec![cached_expectation, uncached_expectation],
            false,
            CachedNonPassPolicy::EmptySelectionLeavesUncachedPending,
        );

        match work {
            GitBackedCacheFilteredCheckWork::DefaultSelectionEmptyWithCachedNonPassReports {
                cached_hits,
            } => assert_eq!(cached_hits.len(), 1),
            GitBackedCacheFilteredCheckWork::EvaluationAllowed { .. } => {
                panic!("cached non-pass must block default selection")
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    fn cache_filtered_work(
        root: &Path,
        source: &TreeSource,
        expectation: ResolvedExpectation,
    ) -> GitBackedCacheFilteredCheckWork {
        cache_filtered_work_with_mode(
            root,
            source,
            vec![expectation],
            false,
            CachedNonPassPolicy::EmptySelectionLeavesUncachedPending,
        )
    }

    fn cache_filtered_work_with_mode(
        root: &Path,
        source: &TreeSource,
        expectations: Vec<ResolvedExpectation>,
        selectors_provided: bool,
        cached_non_pass_policy: CachedNonPassPolicy,
    ) -> GitBackedCacheFilteredCheckWork {
        let mut xpec_state = XpecStateCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let mut diagnostic_log = None;
        select_git_backed_expectations_after_cache(
            GitBackedCacheFilterContext {
                root,
                source,
                xpec_state: &mut xpec_state,
                visible_tree_oid_cache: &mut visible_tree_oid_cache,
                diagnostic_log: &mut diagnostic_log,
            },
            &CheckOptions {
                candidate_expectations: expectations,
                selectors_provided,
                keep_going: false,
                ignore_cooldown: false,
                break_after_tokens: None,
            },
            2,
            cached_non_pass_policy,
        )
        .unwrap()
    }

    fn test_expectation() -> ResolvedExpectation {
        test_expectation_with_identity(1, "abc123", "a")
    }

    fn test_expectation_with_identity(
        number: usize,
        id: &str,
        display_id: &str,
    ) -> ResolvedExpectation {
        ResolvedExpectation {
            number,
            id: id.to_string(),
            display_id: display_id.to_string(),
            question: format!("Does {id} pass?"),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: Option::<ExpectationTarget>::None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: Option::<Cooldown>::None,
        }
    }

    fn test_record(
        expectation: &ResolvedExpectation,
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
            diff_from: Some(crate::config_types::DEFAULT_DIFF_FROM.to_string()),
            diff_from_tree_oid: Some("1234567890abcdef1234567890abcdef12345678".to_string()),
            diff_from_tree_oid_abbrev: Some("1234567".to_string()),
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
