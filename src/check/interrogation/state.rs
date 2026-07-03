use crate::check::core::ERROR_SCOPE_TOO_NARROW;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator::{
    app_server_model_key, evaluator_models, EvaluatorResponseParseCache,
    PromptTemplateOutputDirCache,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::isolation::{NaiveIsolationGuard, NaiveIsolationPolicy};
use crate::scope::{effective_ignore_patterns, visible_scope};
use crate::staged::StagedWorktreeView;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) const IN_PLACE_VISIBLE_TREE_OID: &str = "in-place";
const IN_PLACE_VISIBLE_FILE_COUNT: usize = 1;

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
    pub(crate) visible_tree_oid: &'a str,
    pub(crate) question_context: &'a str,
    pub(crate) diff_base_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
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
    key.push_str(context.visible_tree_oid);
    key.push('\0');
    key.push_str(context.diff_base_tree_oid);
    key.push('\0');
    key.push_str(context.checked_tree_oid);
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
    no_sandbox: bool,
    mode: CheckRuntimeMode<'a>,
}

enum CheckRuntimeMode<'a> {
    Materialized {
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        staged_view: &'a StagedWorktreeView,
        persistent_history: bool,
    },
    InPlace,
}

#[derive(Clone)]
pub(crate) struct CheckTreeContext {
    pub(crate) checked_tree_oid: String,
    pub(crate) against_tree_oid: String,
    pub(crate) checked_file_count: usize,
}

impl<'a> CheckRuntime<'a> {
    pub(crate) fn materialized(
        root: &'a Path,
        staged_view: &'a StagedWorktreeView,
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        config: &'a CheckConfig,
        no_sandbox: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            no_sandbox,
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
        no_sandbox: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            no_sandbox,
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
        no_sandbox: bool,
    ) -> CheckRuntime<'a> {
        // This runtime mode owns the in-place evaluator view: no materialized
        // Git tree, full-project visible scope, stable fake visible-tree
        // metadata, and sessions rooted at the checked directory. Command
        // execution owns cache-free run orchestration, while config validation
        // owns mode compatibility after raw config expansion.
        CheckRuntime {
            root,
            config,
            no_sandbox,
            mode: CheckRuntimeMode::InPlace,
        }
    }

    pub(crate) fn no_sandbox(&self) -> bool {
        self.no_sandbox
    }

    pub(crate) fn is_in_place(&self) -> bool {
        matches!(self.mode, CheckRuntimeMode::InPlace)
    }

    pub(crate) fn evaluator_interrogations_never_hide_files(&self) -> bool {
        // In-place mode is the check mode whose selected expectations are
        // evaluated against the checked directory as-is. Its evaluator
        // interrogations never hide files through q-scope, expectation ignore,
        // scope narrowing, or retry behavior.
        self.is_in_place()
    }

    pub(crate) fn persistent_check_state_root(&self) -> Option<&Path> {
        match self.mode {
            CheckRuntimeMode::Materialized {
                persistent_history, ..
            } => persistent_history.then_some(self.root),
            // In-place mode has no Git-backed persistent check-state target:
            // persisted xpec last-result history is absent, so the Last
            // Results files have no XPECS_DIR to read or update for this
            // runtime.
            CheckRuntimeMode::InPlace => None,
        }
    }

    pub(crate) fn tree_source(&self) -> Option<&TreeSource> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => Some(tree_source),
            CheckRuntimeMode::InPlace => None,
        }
    }

    pub(crate) fn checked_tree_oid(&self) -> &str {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => &tree_context.checked_tree_oid,
            CheckRuntimeMode::InPlace => IN_PLACE_VISIBLE_TREE_OID,
        }
    }

    pub(crate) fn against_tree_oid(&self) -> &str {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => &tree_context.against_tree_oid,
            CheckRuntimeMode::InPlace => IN_PLACE_VISIBLE_TREE_OID,
        }
    }

    pub(crate) fn checked_file_count(&self) -> usize {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_context, .. } => tree_context.checked_file_count,
            CheckRuntimeMode::InPlace => IN_PLACE_VISIBLE_FILE_COUNT,
        }
    }

    pub(crate) fn visible_tree_oid(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        match &self.mode {
            CheckRuntimeMode::Materialized { tree_source, .. } => {
                cache.visible_tree_oid(self.root, tree_source, agent, scope)
            }
            // In-place mode does not construct scoped visible trees. Every
            // requested q-scope induces the same full-project evaluator view
            // rooted at the checked directory.
            CheckRuntimeMode::InPlace => Ok(IN_PLACE_VISIBLE_TREE_OID.to_string()),
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
            // This is fake visible-tree metadata, not a filesystem count. It
            // preserves the Interrogation Policy invariant that q-scope
            // verification runs only when a suggestion induces a smaller
            // visible tree: in in-place mode every q-scope induces the same
            // full-project visible tree, so the exact positive count is
            // irrelevant but must be stable across requested scopes.
            CheckRuntimeMode::InPlace => Ok(IN_PLACE_VISIBLE_FILE_COUNT),
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

    pub(crate) fn fresh_scope_without_persistent_history(&self) -> Option<Vec<String>> {
        match &self.mode {
            CheckRuntimeMode::Materialized {
                persistent_history, ..
            } => (!persistent_history).then(full_scope),
            // In-place mode treats persisted xpec last-result history as
            // absent. This is the Interrogation Policy's "no last pass result
            // with qScope exists" case: fresh interrogations start at full
            // project scope.
            CheckRuntimeMode::InPlace => Some(full_scope()),
        }
    }

    pub(crate) fn session_root_for_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
        visible_tree_oid: &str,
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
                staged_view.materialize_visible_scope(&visible_scope, visible_tree_oid)
            }
            CheckRuntimeMode::InPlace => Ok(self.root.to_path_buf()),
        }
    }
}

pub(crate) struct InterrogationRunState {
    pub(crate) session_isolations: BTreeMap<String, NaiveIsolationGuard>,
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
    pub(crate) session_dynamic_show_expectation_ids: BTreeMap<String, BTreeSet<String>>,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
    pub(crate) prompt_template_output_dir_cache: Arc<PromptTemplateOutputDirCache>,
    isolation_policy: Option<NaiveIsolationPolicy>,
}

impl InterrogationRunState {
    pub(crate) fn new(no_sandbox: bool) -> Result<InterrogationRunState, String> {
        let isolation_policy = if no_sandbox {
            None
        } else {
            Some(NaiveIsolationPolicy::from_env()?)
        };
        Ok(InterrogationRunState {
            session_isolations: BTreeMap::new(),
            thread_sessions_by_prerender_key: BTreeMap::new(),
            thread_sessions_by_rendered_instructions_key: BTreeMap::new(),
            session_base_instructions: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            session_roots_by_id: BTreeMap::new(),
            session_answered_short_ids: BTreeMap::new(),
            session_dynamic_show_expectation_ids: BTreeMap::new(),
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
            prompt_template_output_dir_cache: Arc::new(PromptTemplateOutputDirCache::new()),
            isolation_policy,
        })
    }

    pub(crate) fn models_in_retry_order(&self, agent: &AgentConfig) -> Vec<Option<String>> {
        evaluator_models(agent)
    }

    pub(crate) fn clear_thread_sessions(&mut self) {
        self.session_isolations.clear();
        self.thread_sessions_by_prerender_key.clear();
        self.thread_sessions_by_rendered_instructions_key.clear();
        self.session_base_instructions.clear();
        self.session_instructions.clear();
        self.session_roots_by_id.clear();
        self.session_answered_short_ids.clear();
        self.session_dynamic_show_expectation_ids.clear();
    }

    pub(crate) fn isolate_session_root(
        &mut self,
        session_root: &Path,
    ) -> Result<Option<NaiveIsolationGuard>, String> {
        self.isolation_policy
            .as_mut()
            .map(|policy| policy.isolate(session_root))
            .transpose()
    }

    pub(crate) fn activate_session_root(&mut self, session_id: &str) -> Result<(), String> {
        for (isolated_session_id, isolation) in &mut self.session_isolations {
            if isolated_session_id == session_id {
                isolation.reveal()?;
            } else {
                isolation.hide()?;
            }
        }
        Ok(())
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

    // xpec: G6
    #[test]
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

    #[test]
    fn thread_reuse_key_includes_developer_instruction_tree_inputs() {
        let agent = AgentConfig::default();
        let scope = full_scope();
        let base = evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            thinking: "medium",
            visible_tree_oid: "visible-tree",
            question_context: "instructions",
            diff_base_tree_oid: "base-a",
            checked_tree_oid: "checked-a",
        })
        .unwrap();
        let different_base =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "medium",
                visible_tree_oid: "visible-tree",
                question_context: "instructions",
                diff_base_tree_oid: "base-b",
                checked_tree_oid: "checked-a",
            })
            .unwrap();
        let different_checked =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "medium",
                visible_tree_oid: "visible-tree",
                question_context: "instructions",
                diff_base_tree_oid: "base-a",
                checked_tree_oid: "checked-b",
            })
            .unwrap();
        let different_thinking =
            evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                agent: &agent,
                scope: &scope,
                model: Some("model"),
                thinking: "high",
                visible_tree_oid: "visible-tree",
                question_context: "instructions",
                diff_base_tree_oid: "base-a",
                checked_tree_oid: "checked-a",
            })
            .unwrap();

        assert_ne!(base, different_base);
        assert_ne!(base, different_checked);
        assert_ne!(base, different_thinking);
    }

    #[test]
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

    #[test]
    fn in_place_runtime_uses_full_scope_and_checked_directory() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            hooks: Default::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, false);
        let requested_scope = vec!["src".to_string()];

        assert!(runtime.tree_source().is_none());
        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(runtime.checked_tree_oid(), IN_PLACE_VISIBLE_TREE_OID);
        assert_eq!(runtime.against_tree_oid(), IN_PLACE_VISIBLE_TREE_OID);
        assert_eq!(
            runtime
                .visible_scope(&AgentConfig::default(), &requested_scope)
                .unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime.fresh_scope_without_persistent_history().unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime
                .session_root_for_scope(&AgentConfig::default(), &requested_scope, "in-place")
                .unwrap(),
            root
        );
    }

    #[test]
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
            hooks: Default::default(),
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
            },
            &config,
            false,
        );

        assert!(runtime.persistent_check_state_root().is_none());
        assert_eq!(
            runtime.fresh_scope_without_persistent_history().unwrap(),
            full_scope()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_place_runtime_never_makes_requested_scope_smaller() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::default(),
            hooks: Default::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, false);
        let mut cache = VisibleTreeOidCache::new();
        let agent = AgentConfig::default();
        let src_scope = vec!["src".to_string()];

        assert_eq!(runtime.checked_file_count(), IN_PLACE_VISIBLE_FILE_COUNT);
        assert_eq!(
            runtime
                .visible_file_count(&mut cache, &agent, &full_scope())
                .unwrap(),
            IN_PLACE_VISIBLE_FILE_COUNT
        );
        assert_eq!(
            runtime
                .visible_file_count(&mut cache, &agent, &src_scope)
                .unwrap(),
            IN_PLACE_VISIBLE_FILE_COUNT
        );
        assert_eq!(
            runtime.visible_scope(&agent, &src_scope).unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime
                .visible_tree_oid(&mut cache, &agent, &["missing".to_string()])
                .unwrap(),
            IN_PLACE_VISIBLE_TREE_OID
        );
    }
}
