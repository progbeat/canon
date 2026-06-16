use crate::check::core::ERROR_SCOPE_TOO_NARROW;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator::{
    app_server_model_key, evaluator_models, AppServerModelKey, EvaluatorResponseParseCache,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::isolation::{NaiveIsolationGuard, NaiveIsolationPolicy};
use crate::scope::{effective_ignore_patterns, visible_scope};
use crate::staged::StagedWorktreeView;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn should_retry_full_scope_after_error(error: Option<&str>, scope: &[String]) -> bool {
    if scope == full_scope() {
        return false;
    }
    if error == Some(ERROR_SCOPE_TOO_NARROW) {
        return true;
    }
    false
}

pub(crate) fn evaluator_thread_reuse_key(
    agent: &AgentConfig,
    scope: &[String],
    model: Option<&str>,
    visible_tree_oid: &str,
    expectation_instructions: &str,
) -> Result<String, String> {
    // The glossary's thread invariant is one-way: a reused thread must keep
    // the same evaluator model, visible tree, and expectation instructions.
    // Extra key parts below are stricter developer-instruction inputs that
    // prevent unsafe reuse without allowing cross-model, cross-visible-tree, or
    // cross-instruction reuse.
    let mut key = String::new();
    app_server_model_key(model).push_cache_key_part(&mut key);
    key.push('\0');
    key.push_str(visible_tree_oid);
    key.push('\0');
    key.push_str(&expectation_instructions.len().to_string());
    key.push('\0');
    key.push_str(expectation_instructions);
    key.push('\0');
    for plugin in &agent.plugins {
        key.push_str(&plugin.len().to_string());
        key.push('\0');
        key.push_str(plugin);
        key.push('\0');
    }
    key.push('\0');
    for pattern in effective_ignore_patterns(agent)? {
        key.push_str(&pattern.len().to_string());
        key.push('\0');
        key.push_str(&pattern);
        key.push('\0');
    }
    key.push('\0');
    for path in scope {
        key.push_str(&path.len().to_string());
        key.push('\0');
        key.push_str(path);
        key.push('\0');
    }
    Ok(key)
}

pub(crate) struct CheckRuntime<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    no_sandbox: bool,
    staged_view: &'a StagedWorktreeView,
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
            tree_source,
            tree_context,
            no_sandbox,
            staged_view,
        }
    }

    pub(crate) fn no_sandbox(&self) -> bool {
        self.no_sandbox
    }

    pub(crate) fn visible_tree_oid(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        cache.visible_tree_oid(self.root, self.tree_source, agent, scope)
    }

    pub(crate) fn visible_file_count(
        &self,
        cache: &mut VisibleTreeOidCache,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        cache.visible_file_count(self.root, self.tree_source, agent, scope)
    }

    pub(crate) fn session_root_for_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        // `visible_scope` returns the complete visible-scope pathspec,
        // including configured ignore exclusions. From here down,
        // materialization selects paths solely by applying that pathspec to
        // checked Git entries.
        let visible_scope = visible_scope(agent, scope)?;
        self.staged_view
            .materialize_visible_scope(&visible_scope, visible_tree_oid)
    }
}

pub(crate) struct InterrogationRunState {
    pub(crate) session_isolations: BTreeMap<String, NaiveIsolationGuard>,
    // This is a run-level pool of evaluator threads, not one thread. The
    // reuse key enforces the glossary's model/visible-tree/instructions
    // invariant and also splits on stricter developer-instruction inputs.
    pub(crate) thread_sessions_by_reuse_key: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) session_roots_by_id: BTreeMap<String, PathBuf>,
    pub(crate) unavailable_models: BTreeSet<AppServerModelKey>,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
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
            thread_sessions_by_reuse_key: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            session_roots_by_id: BTreeMap::new(),
            unavailable_models: BTreeSet::new(),
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
            isolation_policy,
        })
    }

    pub(crate) fn available_models(&self, agent: &AgentConfig) -> Vec<Option<String>> {
        evaluator_models(agent)
            .into_iter()
            .filter(|model| !self.model_is_unavailable(model.as_deref()))
            .collect()
    }

    pub(crate) fn model_is_unavailable(&self, model: Option<&str>) -> bool {
        self.unavailable_models
            .contains(&app_server_model_key(model))
    }

    pub(crate) fn mark_model_unavailable(&mut self, model: Option<&str>) {
        self.unavailable_models.insert(app_server_model_key(model));
    }

    pub(crate) fn clear_thread_sessions(&mut self) {
        self.session_isolations.clear();
        self.thread_sessions_by_reuse_key.clear();
        self.session_instructions.clear();
        self.session_roots_by_id.clear();
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
}
