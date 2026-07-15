use crate::check::core::{CheckOptions, ResolvedExpectation};
use crate::check::run::cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::path::Path;

pub(crate) struct GitBackedCacheFilteredCheckWork {
    pub(crate) evaluation_queue: Vec<ResolvedExpectation>,
    pub(crate) cached_hits: Vec<CachedExpectationHit>,
}

pub(crate) struct CachedExpectationHit {
    pub(crate) expectation: ResolvedExpectation,
    pub(crate) hit: CheckCacheHit,
}

pub(crate) struct GitBackedCacheFilterContext<'a, 'log> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a TreeSource,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
}

// Git-backed cache filtering applies the selected-expectation policy before
// evaluation starts. Explicit selections are forced; otherwise only collected
// expectations without a reusable pass result are selected.
//
// This layer only partitions work by cache availability and selectedness. The
// Cache selection reads existing xpec state and may emit bounded runtime-log
// events through the supplied writer, but it does not create a persistent
// state family of its own.
//
// This function receives command-independent `CheckOptions`; CLI/trailer policy
// stays in the command layer before and after this selection step.
pub(crate) fn select_git_backed_expectations_after_cache(
    context: GitBackedCacheFilterContext<'_, '_>,
    options: &CheckOptions,
    now: u64,
) -> Result<GitBackedCacheFilteredCheckWork, String> {
    if options.selectors_provided {
        // Explicit expectation selectors are forced selections. Do not inspect
        // cached results here: a cache hit must not move an explicit candidate
        // out of the evaluator queue.
        return Ok(GitBackedCacheFilteredCheckWork {
            evaluation_queue: options.candidate_expectations.clone(),
            cached_hits: Vec::new(),
        });
    }

    let mut evaluation_queue = Vec::new();
    let mut cached_hits = Vec::new();
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
                include_cooldown: !options.ignore_cooldown,
            },
        )? {
            Some(hit) => {
                if let Some(writer) = context.diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)?;
                }
                cached_hits.push(CachedExpectationHit { expectation, hit });
            }
            None => evaluation_queue.push(expectation),
        }
    }
    Ok(GitBackedCacheFilteredCheckWork {
        evaluation_queue,
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

    #[test] // xpec: CQ,Du
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
                &test_record(&expectation, &scope, "yes", visible_tree_oid),
            )
            .unwrap();

        let work = cache_filtered_work(&root, &source, expectation);
        assert_eq!(work.cached_hits.len(), 1);
        assert!(work.evaluation_queue.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Du
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
        );
        assert!(work.cached_hits.is_empty());
        assert_eq!(work.evaluation_queue.len(), 2);
        assert_eq!(work.evaluation_queue[0].id, "abc123");
        assert_eq!(work.evaluation_queue[1].id, "def456");
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: Du
    fn default_fail_history_leaves_candidates_selected() {
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
        );
        assert!(work.cached_hits.is_empty());
        assert_eq!(work.evaluation_queue.len(), 2);
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
