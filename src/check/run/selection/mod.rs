mod args;
mod cache_select;
mod cooldown;
mod identity;
mod order;

pub(crate) use args::{
    add_check_option_args, matched_os_values, raw_check_options_from_matches,
    resolve_check_options_with_identities,
};
pub(crate) use cache_select::{
    select_expectations_after_cache, CacheFilterContext, CachedExpectationHit, CachedFailureMode,
};
pub(crate) use cooldown::parse_cooldown;
// Selector parsing and matching, including `not:<ID-PREFIX>` exclusions, lives
// in `identity`.
pub(crate) use identity::{
    expectation_identities, select_expectations_with_identities, ExpectationIdentity,
};
pub(crate) use order::order_by_latest_non_pass;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckOptions, CheckRecord, CheckRecordOutcome, CheckResult};
    use crate::config_types::{AgentConfig, CheckConfig, Expectation};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::history::{append_current_history_record_with_cache, HistoryCache};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    fn init_git_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn one_expectation_config() -> CheckConfig {
        CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: AgentConfig::implementation_default(),
            expectations: vec![Expectation {
                q: "Does selector cache reuse avoid unnecessary evaluator work?".to_string(),
                a: "yes".to_string(),
                question_answer_only: true,
                agent: AgentConfig::implementation_default(),
                cooldown: None,
            }],
        }
    }

    #[test]
    fn selector_candidates_with_cached_pass_are_not_evaluated() {
        let root = temp_repo("canon-selector-cache");
        init_git_repo(&root);
        let config = one_expectation_config();
        let identities = expectation_identities(&config).unwrap();
        let expectation = identity::selected_expectation_at(&config, &identities, 0, true).unwrap();
        let options = CheckOptions {
            selected: vec![expectation.clone()],
            selectors_provided: true,
            keep_going: false,
            ignore_cooldown: false,
            break_after_tokens: None,
        };
        let mut history_cache = HistoryCache::default();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let source = TreeSource::Staged;
        let scope = vec![".".to_string()];
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &source, &expectation.agent, &scope)
            .unwrap();
        let record = CheckRecord::current_from_expectation(
            &expectation,
            CheckRecordOutcome {
                result: CheckResult::Pass,
                observed: "yes".to_string(),
                error: None,
                evidence: "cached pass".to_string(),
                scope,
                question_scope_suggestion: None,
                visible_tree_oid,
            },
        )
        .unwrap();
        append_current_history_record_with_cache(
            &root,
            &source,
            &expectation,
            &record,
            &mut history_cache,
            &mut visible_tree_oid_cache,
        )
        .unwrap();
        let mut diagnostic_log = None;

        let check_work = select_expectations_after_cache(
            CacheFilterContext {
                root: &root,
                source: &source,
                history_cache: &mut history_cache,
                visible_tree_oid_cache: &mut visible_tree_oid_cache,
                active_lazy_full_scope_reset_ids: &BTreeSet::new(),
                diagnostic_log: &mut diagnostic_log,
            },
            &options,
            0,
            CachedFailureMode::Continue,
        )
        .unwrap();

        let _ = fs::remove_dir_all(&root);

        assert!(check_work.to_evaluate.is_empty());
        assert_eq!(check_work.cached_hits.len(), 1);
        assert_eq!(check_work.cached_hits[0].expectation.id, expectation.id);
        assert!(!check_work.cached_failure_seen);
    }
}
