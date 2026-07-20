use crate::check::core::ERROR_SCOPE_TOO_NARROW;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator::{
    app_server_model_key, evaluator_models, EvaluatorResponseParseCache, PromptRenderer,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::isolation::{NaiveIsolationGuard, NaiveIsolationPolicy};
use crate::scope::{effective_ignore_patterns, visible_scope};
use crate::staged::StagedWorktreeView;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) fn should_retry_full_scope_after_error(error: Option<&str>, scope: &[String]) -> bool {
    // This is the Interrogation Policy retry predicate only. The check-run
    // follow-up is executed in `src/check/run/execute/expectation.rs`, and
    // `canon ask` applies the same predicate in
    // `src/check/interrogation/query/mod.rs`.
    if scope == full_scope() {
        return false;
    }
    if error == Some(ERROR_SCOPE_TOO_NARROW) {
        return true;
    }
    false
}

pub(crate) struct PrerenderEvaluatorThreadReuseKeyContext<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) view: EvaluatorViewIdentity<'a>,
    pub(crate) question_context: &'a str,
}

pub(crate) enum EvaluatorViewIdentity<'a> {
    InPlace,
    Git {
        visible_tree_oid: Option<&'a str>,
        diff_base_tree_oid: &'a str,
        checked_tree_oid: &'a str,
    },
}

pub(crate) fn evaluator_prerender_thread_reuse_key(
    context: PrerenderEvaluatorThreadReuseKeyContext<'_>,
) -> Result<String, String> {
    // This is the cheap pre-render lookup key. It includes the evaluator model,
    // stable inputs that determine the rendered base/developer instructions, and
    // non-rendered live thread-start context such as tools and visible scope.
    // The turn prompt is deliberately excluded: each evaluator turn supplies its
    // own task input, while a reusable thread keeps only the base/developer
    // instruction context from session startup.
    let mut key = String::new();
    app_server_model_key(context.model).push_cache_key_part(&mut key);
    key.push('\0');
    key.push_str(&context.thinking.len().to_string());
    key.push('\0');
    key.push_str(context.thinking);
    key.push('\0');
    match context.view {
        EvaluatorViewIdentity::InPlace => key.push_str("in-place"),
        EvaluatorViewIdentity::Git {
            visible_tree_oid,
            diff_base_tree_oid,
            checked_tree_oid,
        } => {
            key.push_str("git");
            key.push('\0');
            key.push_str(visible_tree_oid.unwrap_or(""));
            key.push('\0');
            key.push_str(diff_base_tree_oid);
            key.push('\0');
            key.push_str(checked_tree_oid);
        }
    }
    key.push('\0');
    key.push_str(&context.question_context.len().to_string());
    key.push('\0');
    key.push_str(context.question_context);
    key.push('\0');
    for plugin in &context.agent.plugins {
        key.push_str(&plugin.len().to_string());
        key.push('\0');
        key.push_str(plugin);
        key.push('\0');
    }
    key.push('\0');
    for pattern in effective_ignore_patterns(context.agent)? {
        key.push_str(&pattern.len().to_string());
        key.push('\0');
        key.push_str(&pattern);
        key.push('\0');
    }
    key.push('\0');
    for path in context.scope {
        key.push_str(&path.len().to_string());
        key.push('\0');
        key.push_str(path);
        key.push('\0');
    }
    Ok(key)
}

pub(crate) struct RenderedEvaluatorThreadReuseKeyContext<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) base_instructions: &'a str,
    pub(crate) developer_instructions: &'a str,
}

pub(crate) fn evaluator_rendered_thread_reuse_key(
    context: RenderedEvaluatorThreadReuseKeyContext<'_>,
) -> String {
    let mut key = String::new();
    app_server_model_key(context.model).push_cache_key_part(&mut key);
    key.push('\0');
    key.push_str(&context.thinking.len().to_string());
    key.push('\0');
    key.push_str(context.thinking);
    key.push('\0');
    key.push_str(&context.base_instructions.len().to_string());
    key.push('\0');
    key.push_str(context.base_instructions);
    key.push('\0');
    key.push_str(&context.developer_instructions.len().to_string());
    key.push('\0');
    key.push_str(context.developer_instructions);
    key.push('\0');
    for plugin in &context.agent.plugins {
        key.push_str(&plugin.len().to_string());
        key.push('\0');
        key.push_str(plugin);
        key.push('\0');
    }
    key
}

pub(crate) struct CheckRuntime<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    disable_session_isolation: bool,
    mode: CheckRuntimeMode<'a>,
}

enum CheckRuntimeMode<'a> {
    Materialized {
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        staged_view: &'a StagedWorktreeView,
        persistent_history: bool,
    },
    InPlaceCheck {
        persistent_status_history: bool,
    },
    InPlaceTemporaryQuery,
}

#[derive(Clone)]
pub(crate) struct CheckTreeContext {
    pub(crate) checked_tree_oid: String,
    pub(crate) against_tree_oid: String,
    pub(crate) checked_file_count: usize,
    pub(crate) prompt_git_environment: Vec<(OsString, OsString)>,
}

impl<'a> CheckRuntime<'a> {
    pub(crate) fn materialized(
        root: &'a Path,
        staged_view: &'a StagedWorktreeView,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        disable_session_isolation: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            disable_session_isolation,
            mode: CheckRuntimeMode::Materialized {
                tree_source,
                tree_context,
                staged_view,
                persistent_history: true,
            },
        }
    }

    pub(crate) fn materialized_without_persistent_history(
        root: &'a Path,
        staged_view: &'a StagedWorktreeView,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        disable_session_isolation: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            disable_session_isolation,
            mode: CheckRuntimeMode::Materialized {
                tree_source,
                tree_context,
                staged_view,
                persistent_history: false,
            },
        }
    }

    pub(crate) fn in_place(
        root: &'a Path,
        config: &'a CheckConfig,
        persistent_status_history: bool,
    ) -> CheckRuntime<'a> {
        // This runtime mode owns the in-place evaluator view: no materialized
        // Git tree, full-project visibility, and sessions rooted at the
        // checked directory. Command
        // execution queues every selected xpec without cached-result reuse,
        // reads and writes status-specific last-result history only when a
        // canonical persistent state namespace exists. That history has no
        // checkedTreeOid, so even an in-place pass never defines the
        // glossary's Git-tree checkpoint. Config validation owns mode
        // compatibility after raw config expansion.
        CheckRuntime {
            root,
            config,
            disable_session_isolation: true,
            mode: CheckRuntimeMode::InPlaceCheck {
                persistent_status_history,
            },
        }
    }

    pub(crate) fn in_place_temporary_query(
        root: &'a Path,
        config: &'a CheckConfig,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            disable_session_isolation: true,
            mode: CheckRuntimeMode::InPlaceTemporaryQuery,
        }
    }

    pub(crate) fn disable_session_isolation(&self) -> bool {
        self.disable_session_isolation
    }

    pub(crate) fn is_in_place(&self) -> bool {
        matches!(
            self.mode,
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery
        )
    }

    pub(crate) fn evaluator_interrogations_never_hide_files(&self) -> bool {
        // In-place mode evaluates against the checked directory as-is. Its evaluator
        // interrogations never hide files through q-scope, expectation ignore,
        // scope narrowing, or retry behavior.
        self.is_in_place()
    }

    pub(crate) fn persistent_check_state_root(&self) -> Option<&Path> {
        match self.mode {
            CheckRuntimeMode::Materialized {
                persistent_history, ..
            } => persistent_history.then_some(self.root),
            CheckRuntimeMode::InPlaceCheck {
                persistent_status_history,
            } => persistent_status_history.then_some(self.root),
            CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }

    pub(crate) fn tree_source(&self) -> Option<&TreeSource> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => Some(tree_source),
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }

    pub(crate) fn git_checked_tree_oid(&self) -> Option<&str> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => {
                Some(&tree_context.checked_tree_oid)
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }

    pub(crate) fn prompt_git_environment(&self) -> &[(OsString, OsString)] {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => {
                &tree_context.prompt_git_environment
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => &[],
        }
    }

    pub(crate) fn git_against_tree_oid(&self) -> Option<&str> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => {
                Some(&tree_context.against_tree_oid)
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => None,
        }
    }

    pub(crate) fn visible_tree_oid(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => cache
                .visible_tree_oid(self.root, tree_source, agent, scope)
                .map(Some),
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Ok(None)
            }
        }
    }

    pub(crate) fn visible_tree_oid_if_scope_present(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => {
                cache.visible_tree_oid_for_reuse(self.root, tree_source, agent, scope)
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Ok(None)
            }
        }
    }

    pub(crate) fn visible_file_count(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => {
                cache.visible_file_count(self.root, tree_source, agent, scope)
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Ok(0)
            }
        }
    }

    pub(crate) fn num_invisible_files(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized {
                tree_source,
                tree_context,
                ..
            } => {
                let visible_file_count =
                    cache.visible_file_count(self.root, tree_source, agent, scope)?;
                tree_context
                    .checked_file_count
                    .checked_sub(visible_file_count)
                    .ok_or_else(|| "visible file count exceeds checked file count".to_string())
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Ok(0)
            }
        }
    }

    pub(crate) fn visible_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        if self.is_in_place() {
            // In-place q-scopes are normalized to full project scope. CLI
            // parsing rejects `--scope` in this mode, while last-pass q-scopes
            // and evaluator qScopeSuggestion values cannot hide files.
            return Ok(full_scope());
        }
        visible_scope(agent, scope)
    }

    pub(crate) fn scope_without_reusable_q_scope_history(&self) -> Option<Vec<String>> {
        match &self.mode {
            CheckRuntimeMode::Materialized {
                persistent_history, ..
            } => (!persistent_history).then(full_scope),
            // In-place persists pass/fail history for ordering and run
            // classification, but its last-result files intentionally omit
            // Git qScope metadata. Fresh interrogations therefore use the
            // policy's no-reusable-qScope case and start at full scope.
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Some(full_scope())
            }
        }
    }

    pub(crate) fn session_root_for_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
        visible_tree_oid: Option<&str>,
    ) -> Result<PathBuf, String> {
        if self.is_in_place() {
            // In-place evaluator sessions start in the checked directory
            // itself; no scoped materialization is created.
            return Ok(self.root.to_path_buf());
        }
        // `visible_scope` returns the complete visible-scope pathspec,
        // including configured ignore exclusions. From here down,
        // materialization selects paths solely by applying that pathspec to
        // checked Git entries.
        let visible_scope = visible_scope(agent, scope)?;
        match &self.mode {
            CheckRuntimeMode::Materialized { staged_view, .. } => {
                let visible_tree_oid = visible_tree_oid
                    .ok_or("materialized evaluator view is missing its visible tree OID")?;
                staged_view.materialize_visible_scope(&visible_scope, visible_tree_oid)
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Ok(self.root.to_path_buf())
            }
        }
    }

    pub(crate) fn materialized_session_root_path(
        &self,
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized { staged_view, .. } => {
                Ok(staged_view.visible_tree_root_path(visible_tree_oid))
            }
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Err("in-place evaluator view has no materialized session root".to_string())
            }
        }
    }
}

pub(crate) struct InterrogationRunState {
    // One guard owns each distinct moved materialized root. Multiple evaluator
    // threads for the same visible tree share its read-only isolated cwd, so
    // the canonical materialization path cannot be recreated before restore.
    materialized_root_restorations: BTreeMap<PathBuf, NaiveIsolationGuard>,
    // This is a run-level pool of evaluator threads, not one thread. The
    // pre-render key is used before rendering instructions only to avoid
    // rendering solely for the lookup.
    pub(crate) thread_sessions_by_prerender_key: BTreeMap<String, String>,
    // After instructions have been rendered anyway, this key lets identical
    // rendered base/developer instructions reuse a live evaluator thread.
    pub(crate) thread_sessions_by_rendered_instructions_key: BTreeMap<String, String>,
    pub(crate) session_base_instructions: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) session_roots_by_id: BTreeMap<String, PathBuf>,
    pub(crate) session_answered_short_ids: BTreeMap<String, BTreeSet<String>>,
    // xpec: F
    // Per-session memory of expectation IDs whose `canon.show` output reached
    // that evaluator thread; thread reuse filters consult this before reusing
    // the session for any of those expectations.
    pub(crate) session_dynamic_show_expectation_ids: BTreeMap<String, BTreeSet<String>>,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
    pub(crate) prompt_renderer: Arc<PromptRenderer>,
    isolation_policy: Option<NaiveIsolationPolicy>,
}

impl InterrogationRunState {
    pub(crate) fn new(disable_session_isolation: bool) -> Result<InterrogationRunState, String> {
        let isolation_policy = if disable_session_isolation {
            None
        } else {
            Some(NaiveIsolationPolicy::from_env()?)
        };
        Ok(InterrogationRunState {
            materialized_root_restorations: BTreeMap::new(),
            thread_sessions_by_prerender_key: BTreeMap::new(),
            thread_sessions_by_rendered_instructions_key: BTreeMap::new(),
            session_base_instructions: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            session_roots_by_id: BTreeMap::new(),
            session_answered_short_ids: BTreeMap::new(),
            session_dynamic_show_expectation_ids: BTreeMap::new(),
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
            prompt_renderer: Arc::new(PromptRenderer::new()),
            isolation_policy,
        })
    }

    pub(crate) fn models_in_retry_order(&self, agent: &AgentConfig) -> Vec<Option<String>> {
        evaluator_models(agent)
    }

    pub(crate) fn clear_thread_sessions(&mut self) {
        self.materialized_root_restorations.clear();
        self.thread_sessions_by_prerender_key.clear();
        self.thread_sessions_by_rendered_instructions_key.clear();
        self.session_base_instructions.clear();
        self.session_instructions.clear();
        self.session_roots_by_id.clear();
        self.session_answered_short_ids.clear();
        self.session_dynamic_show_expectation_ids.clear();
    }

    pub(crate) fn prepare_materialized_session_root(
        &mut self,
        canonical_root: &Path,
        materialize: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Result<PathBuf, String> {
        if let Some(restoration) = self.materialized_root_restorations.get(canonical_root) {
            return Ok(restoration.path().to_path_buf());
        }
        let materialized_root = materialize()?;
        assert_eq!(
            materialized_root, canonical_root,
            "materialized evaluator root must use its canonical tree path"
        ); // xpec: YY
        let Some(policy) = self.isolation_policy.as_mut() else {
            return Ok(materialized_root);
        };
        let restoration = policy.isolate(&materialized_root)?;
        let session_root = restoration.path().to_path_buf();
        self.materialized_root_restorations
            .insert(materialized_root, restoration);
        Ok(session_root)
    }

    pub(crate) fn answered_short_ids_for_session(&self, session_id: &str) -> Vec<String> {
        self.session_answered_short_ids
            .get(session_id)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn session_has_valid_response(&self, session_id: &str) -> bool {
        self.session_answered_short_ids
            .get(session_id)
            .is_some_and(|ids| !ids.is_empty())
    }

    pub(crate) fn record_session_answered_short_id(&mut self, session_id: &str, short_id: &str) {
        self.session_answered_short_ids
            .entry(session_id.to_string())
            .or_default()
            .insert(short_id.to_string());
    }

    pub(crate) fn session_has_seen_dynamic_show_expectation(
        &self,
        session_id: &str,
        expectation_id: Option<&str>,
    ) -> bool {
        let Some(expectation_id) = expectation_id else {
            return false;
        };
        self.session_dynamic_show_expectation_ids
            .get(session_id)
            .is_some_and(|ids| ids.contains(expectation_id))
    }

    pub(crate) fn record_session_dynamic_show_expectation_ids(
        &mut self,
        session_id: &str,
        expectation_ids: BTreeSet<String>,
    ) {
        if expectation_ids.is_empty() {
            return;
        }
        self.session_dynamic_show_expectation_ids
            .entry(session_id.to_string())
            .or_default()
            .extend(expectation_ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: YY
    fn materialized_sessions_share_one_isolated_root_until_restore() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "canon-shared-isolated-root-{}-{unique}",
            process::id()
        ));
        let canonical_root = root.join("trees/tree-oid");
        let sandbox = root.join("sandbox");
        crate::platform::create_private_dir_all(&canonical_root).unwrap();
        let mut state = InterrogationRunState::new(true).unwrap();
        state.isolation_policy = Some(
            NaiveIsolationPolicy::with_dirs(None, sandbox.clone()).expect("test isolation policy"),
        );
        let materializations = Cell::new(0);

        let first = state
            .prepare_materialized_session_root(&canonical_root, || {
                materializations.set(materializations.get() + 1);
                Ok(canonical_root.clone())
            })
            .unwrap();
        let second = state
            .prepare_materialized_session_root(&canonical_root, || {
                panic!("an isolated canonical root must not be materialized again")
            })
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(materializations.get(), 1);
        assert!(!canonical_root.exists());
        assert!(first.is_dir());
        drop(state);
        assert!(canonical_root.is_dir());
        assert!(sandbox.is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: F
    fn session_dynamic_show_expectation_ids_are_session_scoped() {
        let mut state = InterrogationRunState::new(true).unwrap();
        state.record_session_dynamic_show_expectation_ids(
            "session-a",
            BTreeSet::from(["expectation-a".to_string()]),
        );

        assert!(state.session_has_seen_dynamic_show_expectation("session-a", Some("expectation-a")));
        assert!(
            !state.session_has_seen_dynamic_show_expectation("session-a", Some("expectation-b"))
        );
        assert!(
            !state.session_has_seen_dynamic_show_expectation("session-b", Some("expectation-a"))
        );
        assert!(!state.session_has_seen_dynamic_show_expectation("session-a", None));
    }

    #[test] // xpec: fD
    fn thread_reuse_key_includes_developer_instruction_tree_inputs() {
        let agent = AgentConfig::default();
        let scope = full_scope();
        let base = evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            thinking: "medium",
            view: EvaluatorViewIdentity::Git {
                visible_tree_oid: Some("visible-tree"),
                diff_base_tree_oid: "base-a",
                checked_tree_oid: "checked-a",
            },
            question_context: "instructions",
        })
        .unwrap();
        let different_base =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "medium",
                view: EvaluatorViewIdentity::Git {
                    visible_tree_oid: Some("visible-tree"),
                    diff_base_tree_oid: "base-b",
                    checked_tree_oid: "checked-a",
                },
                question_context: "instructions",
            })
            .unwrap();
        let different_checked =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "medium",
                view: EvaluatorViewIdentity::Git {
                    visible_tree_oid: Some("visible-tree"),
                    diff_base_tree_oid: "base-a",
                    checked_tree_oid: "checked-b",
                },
                question_context: "instructions",
            })
            .unwrap();
        let different_thinking =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "high",
                view: EvaluatorViewIdentity::Git {
                    visible_tree_oid: Some("visible-tree"),
                    diff_base_tree_oid: "base-a",
                    checked_tree_oid: "checked-a",
                },
                question_context: "instructions",
            })
            .unwrap();

        assert_ne!(base, different_base);
        assert_ne!(base, different_checked);
        assert_ne!(base, different_thinking);
    }

    #[test] // xpec: fD
    fn rendered_thread_reuse_key_includes_rendered_instructions() {
        let agent = AgentConfig::default();
        let first = evaluator_rendered_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
            agent: &agent,
            model: Some("model"),
            thinking: "medium",
            base_instructions: "base",
            developer_instructions: "developer-a",
        });
        let second = evaluator_rendered_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
            agent: &agent,
            model: Some("model"),
            thinking: "medium",
            base_instructions: "base",
            developer_instructions: "developer-b",
        });
        let third = evaluator_rendered_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
            agent: &agent,
            model: Some("model"),
            thinking: "high",
            base_instructions: "base",
            developer_instructions: "developer-a",
        });

        assert_ne!(first, second);
        assert_ne!(first, third);
    }

    #[test] // xpec: I4
    fn in_place_runtime_uses_full_scope_and_checked_directory() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let requested_scope = vec!["src".to_string()];

        assert!(runtime.tree_source().is_none());
        assert_eq!(runtime.persistent_check_state_root(), Some(root.as_path()));
        assert!(runtime.git_checked_tree_oid().is_none());
        assert!(runtime.git_against_tree_oid().is_none());
        assert_eq!(
            runtime
                .visible_scope(&AgentConfig::default(), &requested_scope)
                .unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime
                .session_root_for_scope(&AgentConfig::default(), &requested_scope, None)
                .unwrap(),
            root
        );
    }

    #[test] // xpec: 1g,I4,g2
    fn in_place_runtime_without_state_namespace_keeps_no_persistent_history() {
        let root = PathBuf::from("/tmp/canon-in-place-without-state");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, false);

        assert!(runtime.is_in_place());
        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
    }

    #[test] // xpec: Ky
    fn in_place_temporary_query_runtime_has_no_state_root() {
        let root = PathBuf::from("/tmp/canon-in-place-query-runtime");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place_temporary_query(&root, &config);

        assert!(runtime.is_in_place());
        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
    }

    #[test] // xpec: Ky
    fn materialized_runtime_without_persistent_history_has_no_state_root() {
        use crate::git::TreeSource;
        use crate::staged::StagedWorktreeView;
        use std::fs;
        use std::process::{self, Command};
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "canon-runtime-no-history-{}-{}",
            process::id(),
            stamp
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .unwrap();
        fs::write(root.join("README.md"), "hello\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&root)
            .output()
            .unwrap();
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let source = TreeSource::Staged;
        let staged_view = StagedWorktreeView::apply_for_tree_source(&root, source.clone()).unwrap();
        let runtime = CheckRuntime::materialized_without_persistent_history(
            &root,
            &staged_view,
            &source,
            CheckTreeContext {
                checked_tree_oid: "checked".to_string(),
                against_tree_oid: "against".to_string(),
                checked_file_count: 1,
                prompt_git_environment: Vec::new(),
            },
            &config,
            false,
        );

        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.scope_without_reusable_q_scope_history().unwrap(),
            full_scope()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test] // xpec: I4
    fn in_place_runtime_never_makes_requested_scope_smaller() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let mut cache = VisibleTreeOidCache::new();
        let agent = AgentConfig::default();
        let src_scope = vec!["src".to_string()];

        assert_eq!(
            runtime
                .num_invisible_files(&mut cache, &agent, &full_scope())
                .unwrap(),
            0
        );
        assert_eq!(
            runtime
                .num_invisible_files(&mut cache, &agent, &src_scope)
                .unwrap(),
            0
        );
        assert_eq!(
            runtime.visible_scope(&agent, &src_scope).unwrap(),
            full_scope()
        );
        assert!(runtime
            .visible_tree_oid(&mut cache, &agent, &["missing".to_string()])
            .unwrap()
            .is_none());
    }
}
