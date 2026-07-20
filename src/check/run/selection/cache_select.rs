use super::order::order_selected_by_rank_and_latest_fail;
use crate::check::core::{CheckOptions, ResolvedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) struct GitBackedCacheFilteredCheckWork {
    pub(crate) selected_evaluation_queue: Vec<ResolvedExpectation>,
    pub(crate) reused_non_selected_results: Vec<CheckCacheHit>,
}

pub(crate) struct GitBackedCacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// [uf,I4,iZ] This is the complete cached-result selection boundary. Cached
// Result is defined for an expectation and Git state, so both the public
// function and its context require a `TreeSource`; there is deliberately no
// in-place cache lookup API. Explicit Git-backed selections are forced but
// still ordered. In default mode, cache hits become reused, non-selected
// results; only cache misses become Selected expectations and enter the ordered
// evaluator queue. The reused results are report bookkeeping, not evaluations,
// and therefore are not members of the check-order sequence.
//
// Selection reads existing xpec state and may emit bounded runtime-log events
// through the supplied writer, but it does not create a persistent state family
// of its own.
//
// This function receives command-independent `CheckOptions`; CLI/trailer policy
// stays in the command layer before and after this selection step.
pub(crate) fn select_and_order_git_backed_expectations(
    context: GitBackedCacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
) -> Result<GitBackedCacheFilteredCheckWork, String> {
    let (selected_evaluation_queue, reused_non_selected_results) = if options.selectors_provided {
        // [E] A forced selection may still have a cached result; Cached Result
        // defines that result, while Selected Expectations requires evaluation
        // anyway. Do not reuse the cached result or move the explicitly selected
        // xpec out of the evaluator queue.
        (options.candidate_expectations.clone(), Vec::new())
    } else {
        let mut selected_evaluation_queue = Vec::new();
        let mut reused_non_selected_results = Vec::new();
        for expectation in options.candidate_expectations.clone() {
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
                    include_cooldown: true,
                },
            )? {
                Some(hit) => {
                    if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                        write_cache_hit(writer, &hit)?;
                    }
                    reused_non_selected_results.push(hit);
                }
                None => selected_evaluation_queue.push(expectation),
            }
        }
        (selected_evaluation_queue, reused_non_selected_results)
    };
    let selected_evaluation_queue = order_selected_by_rank_and_latest_fail(
        context.root,
        selected_evaluation_queue,
        &mut *context.xpec_state,
        |expectation| expectation,
    )?;
    Ok(GitBackedCacheFilteredCheckWork {
        selected_evaluation_queue,
        reused_non_selected_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckRecord, CheckResult};
    use crate::config_types::{AgentConfig, Cooldown, ExpectationTarget};
    use crate::hash::full_scope;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: uf,E
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
                Some(&checked_tree_oid),
                &expectation,
                &test_record(&expectation, &scope, "yes", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work(&root, &source, expectation);
        assert_eq!(work.reused_non_selected_results.len(), 1);
        assert!(work.selected_evaluation_queue.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: E,IJ
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
                Some(&checked_tree_oid),
                &cached_expectation,
                &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work_with_mode(
            &root,
            &source,
            vec![uncached_expectation, cached_expectation],
            true,
        );
        assert!(work.reused_non_selected_results.is_empty());
        assert_eq!(work.selected_evaluation_queue.len(), 2);
        assert_eq!(work.selected_evaluation_queue[0].id, "abc123");
        assert_eq!(work.selected_evaluation_queue[1].id, "def456");
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: E,uf
    fn same_tree_fail_history_is_not_a_cached_result() {
        let root = git_project("default-fail-history-does-not-cache");
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
                Some(&checked_tree_oid),
                &cached_expectation,
                &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work_with_mode(
            &root,
            &source,
            vec![cached_expectation, uncached_expectation],
            false,
        );
        assert!(work.reused_non_selected_results.is_empty());
        assert_eq!(work.selected_evaluation_queue.len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    fn cache_filtered_work(
        root: &Path,
        source: &TreeSource,
        expectation: ResolvedExpectation,
    ) -> GitBackedCacheFilteredCheckWork {
        cache_filtered_work_with_mode(root, source, vec![expectation], false)
    }

    fn cache_filtered_work_with_mode(
        root: &Path,
        source: &TreeSource,
        expectations: Vec<ResolvedExpectation>,
        selectors_provided: bool,
    ) -> GitBackedCacheFilteredCheckWork {
        let mut xpec_state = XpecStateCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let mut diagnostic_log = None;
        select_and_order_git_backed_expectations(
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
            },
            2,
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
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
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
            to: crate::config_types::ExpectationTo::Agent,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: observed.to_string(),
            error: None,
            evidence: Some("evidence".to_string()),
            scope: scope.to_vec(),
            question_scope_suggestion: Some(scope.to_vec()),
            visible_tree_oid: Some(visible_tree_oid),
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
        // xpec: uf,E
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
