mod cleanup;
mod last_result;

use crate::check::{CheckRecord, CheckResult, Cooldown, SelectedExpectation};
use crate::git::{resolve_git_path, TreeSource, VisibleTreeOidCache};
use crate::state_paths::CANON_XPECS_DIR_GIT_PATH;
use crate::time::parse_record_timestamp;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) use cleanup::{active_expectation_ids_from_identities, cleanup_stale_xpec_dirs};
use last_result::{check_record_from_last_result, pass_record_from_cooldown_result};
pub(crate) use last_result::{LastResult, LastResultStatus};

#[derive(Default)]
pub(crate) struct XpecStateCache {
    absent_persistent_history_roots: BTreeSet<PathBuf>,
    xpecs_dirs: BTreeMap<PathBuf, PathBuf>,
    xpec_dirs: BTreeMap<(PathBuf, String), PathBuf>,
    last_results: BTreeMap<LastResultCacheKey, Option<LastResult>>,
}

type LastResultCacheKey = (PathBuf, String, LastResultStatus);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedResultStatus {
    Pass,
    Fail,
}

pub(crate) struct CachedLastResultHit {
    pub(crate) result: LastResult,
    pub(crate) status: CachedResultStatus,
    pub(crate) kind: CachedLastResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedLastResultKind {
    SameTree,
    Cooldown,
}

impl XpecStateCache {
    pub(crate) fn with_absent_persistent_history(root: &Path) -> XpecStateCache {
        let mut cache = XpecStateCache::default();
        cache
            .absent_persistent_history_roots
            .insert(root.to_path_buf());
        cache
    }

    pub(crate) fn persistent_history_is_absent(&self, root: &Path) -> bool {
        self.absent_persistent_history_roots.contains(root)
    }

    pub(crate) fn xpecs_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.xpecs_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = resolve_git_path(root, CANON_XPECS_DIR_GIT_PATH)?;
        self.xpecs_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn xpec_dir(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.xpec_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = self.xpecs_dir(root)?.join(&expectation.id);
        self.xpec_dirs.insert(key, path.clone());
        Ok(path)
    }
}

pub(crate) fn snapshot_pass_ids(
    root: &Path,
    expectations: &[SelectedExpectation],
    cache: &mut XpecStateCache,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for expectation in expectations {
        if cache.read_last_pass(root, expectation)?.is_some() {
            ids.insert(expectation.id.clone());
        }
    }
    Ok(ids)
}

pub(crate) struct CachedLastResultLookup {
    pub(crate) now: u64,
    pub(crate) include_same_tree: bool,
    pub(crate) include_cooldown: bool,
}

pub(crate) fn cached_last_result_for_expectation(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedLastResultLookup,
) -> Result<Option<CachedLastResultHit>, String> {
    // This is the Cached Result implementation for ordinary Git-backed runs.
    // The in-place command path never calls it; in-place's separate
    // compatibility validator can reject configured cooldown because that mode
    // has no persistent last-result history to query.
    // Cached results are answers for the checked visible tree. `diff-from`
    // only chooses the left-hand tree for prompt-rendered Git diffs during
    // fresh evaluator work, so it is not part of cache identity.
    if lookup.include_same_tree {
        if let Some((result, status)) = same_tree_last_result(
            root,
            source,
            expectation,
            state_cache,
            visible_tree_oid_cache,
        )? {
            return Ok(Some(CachedLastResultHit {
                result,
                status,
                kind: CachedLastResultKind::SameTree,
            }));
        }
    }
    if lookup.include_cooldown {
        if let Some(result) = cooldown_last_result(root, expectation, state_cache, lookup.now)? {
            return Ok(Some(CachedLastResultHit {
                result,
                status: CachedResultStatus::Pass,
                kind: CachedLastResultKind::Cooldown,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn refresh_reused_same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    mut hit: CachedLastResultHit,
) -> Result<CachedLastResultHit, String> {
    if hit.kind == CachedLastResultKind::SameTree {
        // The cached-result rule has already selected this hit. This write is
        // only the Last Results bookkeeping required when a same-tree result is
        // reused.
        let current_checked_tree_oid = source.tree_oid_for_prompt_diff(root)?;
        hit.result = state_cache.refresh_last_result_for_checked_tree(
            root,
            &current_checked_tree_oid,
            expectation,
            &hit.result,
        )?;
    }
    Ok(hit)
}

pub(crate) fn check_record_from_cached_result(
    expectation: &SelectedExpectation,
    hit: &CachedLastResultHit,
) -> CheckRecord {
    match hit.status {
        CachedResultStatus::Pass if hit.kind == CachedLastResultKind::Cooldown => {
            pass_record_from_cooldown_result(expectation, &hit.result)
        }
        CachedResultStatus::Pass | CachedResultStatus::Fail => {
            check_record_from_last_result(expectation, &hit.result)
        }
    }
}

pub(crate) fn latest_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    cache: &mut XpecStateCache,
) -> Result<Option<u64>, String> {
    // Human-review results are persisted as `last-error.json`: evaluator
    // schema errors such as ScopeTooNarrow are not pass/fail answers, and
    // `CheckRecord::requires_human_review` is defined by the same `error`
    // field. Ordering therefore treats fail and error status files as the
    // complete non-pass history.
    let fail = cache.read_last_fail(root, expectation)?;
    let error = cache.read_last_error(root, expectation)?;
    Ok([fail, error]
        .into_iter()
        .flatten()
        .filter_map(|result| parse_record_timestamp(&result.response_timestamp))
        .max())
}

fn same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<(LastResult, CachedResultStatus)>, String> {
    let resolver = visible_tree_oid_cache.reuse_resolver(root, source)?;
    for (last_status, cached_status) in [
        (LastResultStatus::Fail, CachedResultStatus::Fail),
        (LastResultStatus::Pass, CachedResultStatus::Pass),
    ] {
        let Some(result) = state_cache.read_last_result(root, expectation, last_status)? else {
            continue;
        };
        let Some(stored_visible_tree_oid) = result.visible_tree_oid.as_deref() else {
            continue;
        };
        // The cached-result rule compares the stored visibleTreeOid with the
        // current visible tree built from that same stored visible-scope
        // pathspec. Reconstructing a q-scope here would make current agent
        // ignores part of history reuse.
        let Some(current_visible_tree_oid) =
            resolver.visible_tree_oid_for_visible_scope_pathspec(&result.visible_scope)?
        else {
            continue;
        };
        if current_visible_tree_oid == stored_visible_tree_oid {
            return Ok(Some((result, cached_status)));
        }
    }
    Ok(None)
}

fn cooldown_last_result(
    root: &Path,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
) -> Result<Option<LastResult>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let pass = cooldown_last_result_for_status(
        root,
        expectation,
        state_cache,
        now,
        cooldown,
        LastResultStatus::Pass,
        CheckResult::Pass,
    )?;
    let fail = cooldown_last_result_for_status(
        root,
        expectation,
        state_cache,
        now,
        cooldown,
        LastResultStatus::Fail,
        CheckResult::Fail,
    )?;
    Ok([pass, fail]
        .into_iter()
        .flatten()
        .max_by_key(|result| parse_record_timestamp(&result.response_timestamp).unwrap_or(0)))
}

fn cooldown_last_result_for_status(
    root: &Path,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
    cooldown: Cooldown,
    last_status: LastResultStatus,
    check_result: CheckResult,
) -> Result<Option<LastResult>, String> {
    let Some(duration) = cooldown.duration_for(check_result) else {
        return Ok(None);
    };
    let Some(result) = state_cache.read_last_result(root, expectation, last_status)? else {
        return Ok(None);
    };
    let Some(response_timestamp) = parse_record_timestamp(&result.response_timestamp) else {
        return Ok(None);
    };
    if now.saturating_sub(response_timestamp) >= duration {
        return Ok(None);
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use crate::check::{CheckRecord, CheckResult, Cooldown, SelectedExpectation};
    use crate::config_types::{AgentConfig, ExpectationTarget};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::hash::full_scope;
    use crate::xpec_state::{
        cached_last_result_for_expectation, check_record_from_cached_result,
        refresh_reused_same_tree_last_result, CachedLastResultKind, CachedLastResultLookup,
        CachedResultStatus, LastResult, LastResultStatus, XpecStateCache,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn last_result_files_use_status_dependent_fields_and_last_json_follows_error() {
        let root = git_project("last-result-status-fields");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let scope = full_scope();

        let pass = test_record(&expectation, &scope, "yes", None);
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &pass)
            .unwrap();
        let pass_json = read_json(&root, &expectation.id, "last-pass.json");
        assert_eq!(pass_json["status"], "pass");
        assert!(pass_json.get("id").is_none());
        assert!(pass_json.get("displayId").is_none());
        assert_eq!(pass_json["response"]["qScopeSuggestion"], json!(scope));
        assert_eq!(pass_json["checkedTreeOid"], "checked-tree");
        assert_eq!(pass_json["visibleTreeOid"], "visible-tree");
        assert!(!last_result_path(&root, &expectation.display_id, "last-pass.json").exists());

        let fail = test_record(&expectation, &scope, "no", None);
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &fail)
            .unwrap();
        let fail_json = read_json(&root, &expectation.id, "last-fail.json");
        assert_eq!(fail_json["status"], "fail");
        assert!(fail_json.get("checkedTreeOid").is_none());
        assert_eq!(fail_json["visibleTreeOid"], "visible-tree");

        let error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
            .unwrap();
        let error_json = read_json(&root, &expectation.id, "last-error.json");
        assert_eq!(error_json["status"], "error");
        assert!(error_json.get("checkedTreeOid").is_none());
        assert!(error_json.get("visibleTreeOid").is_none());

        let last_json = read_json(&root, &expectation.id, "last.json");
        assert_eq!(last_json, error_json);
        assert!(!last_result_path(&root, &expectation.id, "stored-q-scope.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_error_is_not_a_cached_result() {
        let root = git_project("last-error-not-cached");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let scope = full_scope();

        let error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
            .unwrap();

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap();

        assert!(hit.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_result_unexpected_answer_uses_fail_status() {
        let root = git_project("last-result-unexpected-answer");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let scope = full_scope();

        let invalid_answer = test_record(&expectation, &scope, "not usable", None);
        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &invalid_answer)
            .unwrap();

        let fail_json = read_json(&root, &expectation.id, "last-fail.json");
        assert_eq!(fail_json["status"], "fail");
        assert_eq!(fail_json["response"]["answer"], "not usable");
        assert!(fail_json.get("checkedTreeOid").is_none());
        assert_eq!(fail_json["visibleTreeOid"], "visible-tree");
        assert!(!last_result_path(&root, &expectation.id, "last-error.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_pass_q_scope_ignores_fail_and_error_results() {
        let root = git_project("last-pass-q-scope-only");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let pass_scope = vec!["src/pass.rs".to_string()];
        let fail_scope = vec!["src/fail.rs".to_string()];
        let error_scope = vec!["src/error.rs".to_string()];

        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(LastResultStatus::Fail, &fail_scope, "1970-01-01T00:00:02Z"),
        );
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(
                LastResultStatus::Error,
                &error_scope,
                "1970-01-01T00:00:03Z",
            ),
        );

        assert!(cache
            .read_last_pass_q_scope(&root, &expectation)
            .unwrap()
            .is_none());

        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(LastResultStatus::Pass, &pass_scope, "1970-01-01T00:00:01Z"),
        );

        let mut cache = XpecStateCache::default();
        assert_eq!(
            cache
                .read_last_pass_q_scope(&root, &expectation)
                .unwrap()
                .unwrap(),
            pass_scope
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn absent_persistent_history_does_not_read_existing_last_results() {
        let root = git_project("absent-persistent-history");
        let expectation = test_expectation();
        let scope = full_scope();
        let fail = test_record(&expectation, &scope, "no", None);
        let mut writer = XpecStateCache::default();
        writer
            .write_last_result_for_record(&root, "checked-tree", &expectation, &fail)
            .unwrap();

        let mut absent_history = XpecStateCache::with_absent_persistent_history(&root);

        assert!(last_result_path(&root, &expectation.id, "last-fail.json").exists());
        assert!(absent_history
            .read_last_fail(&root, &expectation)
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_error_preserves_response_suggestion() {
        let root = git_project("last-error-suggestion-separate");
        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let scope = vec!["src/too-narrow.rs".to_string()];
        let suggestion = vec!["src/needed.rs".to_string()];
        let mut error = test_record(
            &expectation,
            &scope,
            "ScopeTooNarrow",
            Some("ScopeTooNarrow"),
        );
        error.question_scope_suggestion = Some(suggestion.clone());

        cache
            .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
            .unwrap();

        let error_json = read_json(&root, &expectation.id, "last-error.json");
        assert_eq!(error_json["qScope"], json!(scope));
        assert_eq!(
            error_json["response"]["qScopeSuggestion"],
            json!(suggestion)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_pass_stores_applied_scope_and_response_suggestion_separately() {
        let root = git_project("last-pass-suggestion-separate");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/narrow.rs"), "narrow\n").unwrap();
        fs::write(root.join("src/extra.rs"), "extra\n").unwrap();
        git(&root, &["add", "src/narrow.rs", "src/extra.rs"]);

        let expectation = test_expectation();
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let persisted_scope = vec!["src/narrow.rs".to_string()];
        let mut record = test_record(&expectation, &persisted_scope, "yes", None);
        record.visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(
                &root,
                &TreeSource::Staged,
                &expectation.agent,
                &persisted_scope,
            )
            .unwrap();
        record.question_scope_suggestion = Some(full_scope());

        XpecStateCache::default()
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
            .unwrap();

        let pass_json = read_json(&root, &expectation.id, "last-pass.json");
        assert_eq!(pass_json["qScope"], json!(persisted_scope));
        assert_eq!(
            pass_json["response"]["qScopeSuggestion"],
            json!(full_scope())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_record_preserves_response_question_scope_suggestion() {
        let root = git_project("cached-record-suggestion");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/narrow.rs"), "narrow\n").unwrap();
        fs::write(root.join("src/extra.rs"), "extra\n").unwrap();
        git(&root, &["add", "src/narrow.rs", "src/extra.rs"]);

        let expectation = test_expectation();
        let q_scope = vec!["src/narrow.rs".to_string()];
        let suggestion = vec!["src/extra.rs".to_string()];
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut record = test_record(&expectation, &q_scope, "yes", None);
        record.visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &q_scope)
            .unwrap();
        record.question_scope_suggestion = Some(suggestion.clone());

        let mut cache = XpecStateCache::default();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
            .unwrap();

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();
        let cached_record = check_record_from_cached_result(&expectation, &hit);

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        assert_eq!(cached_record.scope, q_scope);
        assert_eq!(cached_record.question_scope_suggestion, Some(suggestion));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn last_result_response_preserves_question_scope_suggestion() {
        let root = git_project("last-result-no-suggestion");
        let expectation = test_expectation();
        let scope = full_scope();
        let mut record = test_record(&expectation, &scope, "yes", None);
        record.question_scope_suggestion = Some(full_scope());

        XpecStateCache::default()
            .write_last_result_for_record(&root, "checked-tree", &expectation, &record)
            .unwrap();

        let pass_json = read_json(&root, &expectation.id, "last-pass.json");
        assert_eq!(
            pass_json["response"]["qScopeSuggestion"],
            json!(full_scope())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_tree_pass_reuses_last_pass_when_only_hidden_files_change() {
        let root = git_project("same-tree-pass-hidden-change");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "a\n").unwrap();
        fs::write(root.join("src/b.rs"), "b\n").unwrap();
        git(&root, &["add", "src/a.rs", "src/b.rs"]);

        let mut expectation = test_expectation();
        expectation.target = Some(ExpectationTarget::Diff);
        let mut cache = XpecStateCache::default();
        let q_scope = vec!["src/a.rs".to_string()];
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut record = test_record(&expectation, &q_scope, "yes", None);
        record.visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &q_scope)
            .unwrap();
        record.question_scope_suggestion = Some(full_scope());
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
            .unwrap();

        fs::write(root.join("src/b.rs"), "changed\n").unwrap();
        git(&root, &["add", "src/b.rs"]);
        let refreshed_checked_tree_oid =
            TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        assert_ne!(checked_tree_oid, refreshed_checked_tree_oid);

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        assert_eq!(hit.result.q_scope, q_scope);

        let hit = refresh_reused_same_tree_last_result(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            hit,
        )
        .unwrap();

        assert_eq!(
            hit.result.checked_tree_oid.as_deref(),
            Some(refreshed_checked_tree_oid.as_str())
        );
        let pass_json = read_json(&root, &expectation.id, "last-pass.json");
        assert_eq!(pass_json["checkedTreeOid"], refreshed_checked_tree_oid);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_tree_result_does_not_reconstruct_q_scope_from_current_agent_ignore() {
        let root = git_project("same-tree-no-current-ignore-reconstruction");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "a\n").unwrap();
        fs::write(root.join("src/b.rs"), "b\n").unwrap();
        git(&root, &["add", "src/a.rs", "src/b.rs"]);

        let mut expectation = test_expectation();
        expectation.agent.ignore = vec!["src/b.rs".to_string()];
        let mut cache = XpecStateCache::default();
        let scope = full_scope();
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut record = test_record(&expectation, &scope, "yes", None);
        record.visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
            .unwrap();

        fs::write(root.join("src/b.rs"), "changed\n").unwrap();
        git(&root, &["add", "src/b.rs"]);
        expectation.agent.ignore.clear();

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn same_tree_result_ignores_diff_from_prompt_direction() {
        let root = git_project("same-tree-ignores-diff-from");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "a\n").unwrap();
        git(&root, &["add", "src/a.rs"]);

        let mut expectation = test_expectation();
        expectation.target = Some(ExpectationTarget::Diff);
        let mut cache = XpecStateCache::default();
        let scope = full_scope();
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut record = test_record(&expectation, &scope, "yes", None);
        record.visible_tree_oid = VisibleTreeOidCache::new()
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
            .unwrap();

        expectation.diff_from = crate::config_types::AGAINST_TREE_DIFF_FROM.to_string();
        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        assert_eq!(hit.result.answer(), Some("yes"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn new_answer_status_keeps_opposite_answer_result() {
        let root = git_project("new-answer-keeps-opposite");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "a\n").unwrap();
        fs::write(root.join("src/b.rs"), "b\n").unwrap();
        git(&root, &["add", "src/a.rs", "src/b.rs"]);

        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let fail_scope = vec!["src/a.rs".to_string()];
        let pass_scope = full_scope();
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut oid_cache = VisibleTreeOidCache::new();

        let mut fail = test_record(&expectation, &fail_scope, "no", None);
        fail.visible_tree_oid = oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &fail_scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
            .unwrap();

        let mut pass = test_record(&expectation, &pass_scope, "yes", None);
        pass.visible_tree_oid = oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &pass_scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
            .unwrap();

        assert!(last_result_path(&root, &expectation.id, "last-fail.json").exists());
        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Fail);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn new_fail_keeps_last_pass_checkpoint_and_reusable_same_tree_pass() {
        let root = git_project("new-fail-keeps-last-pass");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "a\n").unwrap();
        fs::write(root.join("src/b.rs"), "b\n").unwrap();
        git(&root, &["add", "src/a.rs", "src/b.rs"]);

        let expectation = test_expectation();
        let mut cache = XpecStateCache::default();
        let pass_scope = vec!["src/a.rs".to_string()];
        let fail_scope = vec!["src/b.rs".to_string()];
        let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
        let mut oid_cache = VisibleTreeOidCache::new();

        let mut pass = test_record(&expectation, &pass_scope, "yes", None);
        pass.visible_tree_oid = oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &pass_scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
            .unwrap();

        let mut fail = test_record(&expectation, &fail_scope, "no", None);
        fail.visible_tree_oid = oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &fail_scope)
            .unwrap();
        cache
            .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
            .unwrap();

        let last_pass = cache.read_last_pass(&root, &expectation).unwrap().unwrap();
        assert_eq!(
            last_pass.checked_tree_oid.as_deref(),
            Some(checked_tree_oid.as_str())
        );
        assert!(last_result_path(&root, &expectation.id, "last-fail.json").exists());

        fs::write(root.join("src/b.rs"), "changed\n").unwrap();
        git(&root, &["add", "src/b.rs"]);

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: true,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::SameTree);
        assert_eq!(
            hit.result.checked_tree_oid.as_deref(),
            Some(checked_tree_oid.as_str())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_backed_cooldown_can_reuse_older_pass_when_fail_is_newer() {
        let root = git_project("cooldown-older-pass");
        let mut expectation = test_expectation();
        expectation.cooldown = Some(Cooldown {
            pass_seconds: Some(10),
            fail_seconds: None,
        });
        let mut cache = XpecStateCache::default();
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(
                LastResultStatus::Pass,
                &["src/pass.rs".to_string()],
                "1970-01-01T00:00:01Z",
            ),
        );
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(
                LastResultStatus::Fail,
                &["src/fail.rs".to_string()],
                "1970-01-01T00:00:02Z",
            ),
        );

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: false,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
        assert_eq!(hit.result.status, LastResultStatus::Pass);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_backed_cooldown_can_reuse_older_fail_when_pass_is_newer() {
        let root = git_project("cooldown-older-fail");
        let mut expectation = test_expectation();
        expectation.cooldown = Some(Cooldown {
            pass_seconds: None,
            fail_seconds: Some(10),
        });
        let mut cache = XpecStateCache::default();
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(
                LastResultStatus::Fail,
                &["src/fail.rs".to_string()],
                "1970-01-01T00:00:01Z",
            ),
        );
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result(
                LastResultStatus::Pass,
                &["src/pass.rs".to_string()],
                "1970-01-01T00:00:02Z",
            ),
        );

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 2,
                include_same_tree: false,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
        assert_eq!(hit.result.status, LastResultStatus::Fail);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_backed_cooldown_uses_response_timestamp_when_both_statuses_match() {
        let root = git_project("cooldown-response-timestamp");
        let mut expectation = test_expectation();
        expectation.cooldown = Some(Cooldown {
            pass_seconds: Some(10),
            fail_seconds: Some(10),
        });
        let mut cache = XpecStateCache::default();
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result_with_response_timestamp(
                LastResultStatus::Pass,
                &["src/pass.rs".to_string()],
                "1970-01-01T00:00:01Z",
                "1970-01-01T00:00:09Z",
            ),
        );
        write_last_result_fixture(
            &root,
            &expectation,
            test_last_result_with_response_timestamp(
                LastResultStatus::Fail,
                &["src/fail.rs".to_string()],
                "1970-01-01T00:00:04Z",
                "1970-01-01T00:00:02Z",
            ),
        );

        let hit = cached_last_result_for_expectation(
            &root,
            &TreeSource::Staged,
            &expectation,
            &mut cache,
            &mut VisibleTreeOidCache::new(),
            CachedLastResultLookup {
                now: 5,
                include_same_tree: false,
                include_cooldown: true,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.status, CachedResultStatus::Pass);
        assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
        assert_eq!(hit.result.status, LastResultStatus::Fail);
        let _ = fs::remove_dir_all(root);
    }

    fn test_expectation() -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: "abc123".to_string(),
            display_id: "a".to_string(),
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            diff_from_configured: false,
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
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

    fn test_record(
        expectation: &SelectedExpectation,
        scope: &[String],
        observed: &str,
        error: Option<&str>,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: crate::time::format_record_timestamp(1),
            number: expectation.number,
            result: CheckResult::from_expected_answer(&expectation.expected_answer, observed),
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "evidence".to_string(),
            scope: scope.to_vec(),
            question_scope_suggestion: Some(scope.to_vec()),
            visible_tree_oid: "visible-tree".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
        }
    }

    fn test_last_result(
        status: LastResultStatus,
        scope: &[String],
        updated_timestamp: &str,
    ) -> LastResult {
        test_last_result_with_response_timestamp(
            status,
            scope,
            "1970-01-01T00:00:01Z",
            updated_timestamp,
        )
    }

    fn test_last_result_with_response_timestamp(
        status: LastResultStatus,
        scope: &[String],
        response_timestamp: &str,
        updated_timestamp: &str,
    ) -> LastResult {
        let response = match status {
            LastResultStatus::Pass => json!({
                "answer": "yes",
                "evidence": "evidence",
                "qScopeSuggestion": scope,
            }),
            LastResultStatus::Fail => json!({
                "answer": "no",
                "evidence": "evidence",
                "qScopeSuggestion": scope,
            }),
            LastResultStatus::Error => json!({
                "error": "ScopeTooNarrow",
                "evidence": "evidence",
                "qScopeSuggestion": scope,
            }),
        };
        LastResult {
            response_timestamp: response_timestamp.to_string(),
            updated_timestamp: updated_timestamp.to_string(),
            status,
            response,
            q_scope: scope.to_vec(),
            visible_scope: scope.to_vec(),
            checked_tree_oid: (status == LastResultStatus::Pass)
                .then(|| "checked-tree".to_string()),
            visible_tree_oid: matches!(status, LastResultStatus::Pass | LastResultStatus::Fail)
                .then(|| "visible-tree".to_string()),
        }
    }

    fn read_json(root: &Path, id: &str, file_name: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(last_result_path(root, id, file_name)).unwrap())
            .unwrap()
    }

    fn write_last_result_fixture(
        root: &Path,
        expectation: &SelectedExpectation,
        result: LastResult,
    ) {
        let file_name = match result.status {
            LastResultStatus::Pass => "last-pass.json",
            LastResultStatus::Fail => "last-fail.json",
            LastResultStatus::Error => "last-error.json",
        };
        let path = last_result_path(root, &expectation.id, file_name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string(&result).unwrap()).unwrap();
    }

    fn last_result_path(root: &Path, id: &str, file_name: &str) -> PathBuf {
        root.join(".git")
            .join("canon")
            .join("xpecs")
            .join(id)
            .join(file_name)
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
        let output = Command::new("git")
            .arg("init")
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
