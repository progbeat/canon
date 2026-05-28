use crate::check_types::{CheckRecord, ObservedAnswerState, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator_config::app_server_model_key;
use crate::evaluator_response_cache::EvaluatorResponseParseCache;
use crate::evaluator_turn::evaluator_models;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_reuse::latest_history_scope_with_cache;
use crate::scope::effective_ignore_patterns;
use crate::staged_worktree::StagedWorktreeView;
use crate::visible_tree_oid::VisibleTreeOidCache;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn should_retry_full_scope_after_restricted_insufficient_evidence(
    record: &CheckRecord,
    scope: &[String],
) -> bool {
    scope != full_scope()
        && ObservedAnswerState::from_observed(&record.observed)
            == ObservedAnswerState::InsufficientEvidence
}

pub(crate) fn initial_visible_scope_for_expectation(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Vec<String>, String> {
    // Glossary visible-scope selection starts from the latest verified q-scope
    // stored in answer history. If no q-scope is stored, fresh interrogation
    // starts from full project scope. This vector is the inclusion side of the
    // visible scope; `session_root_for_scope` and visibleTreeOid hashing apply
    // the expectation agent's normalized ignore patterns as exclusions last.
    //
    // Stored scopes are trusted because they are written only after independent
    // q-scope verification. Even if a stored q-scope's paths are absent in the
    // current tree, the first interrogation still uses that q-scope; restricted
    // insufficient-evidence is the only policy that widens it to full scope.
    Ok(
        latest_history_scope_with_cache(root, &expectation.agent, expectation, history_cache)?
            .unwrap_or_else(full_scope),
    )
}

pub(crate) fn evaluator_thread_reuse_key(
    agent: &AgentConfig,
    scope: &[String],
    model: Option<&str>,
    visible_tree_oid: &str,
) -> String {
    let mut key = String::new();
    key.push_str(model.unwrap_or("<default>"));
    key.push('\0');
    key.push_str(visible_tree_oid);
    key.push('\0');
    for plugin in &agent.plugins {
        key.push_str(&plugin.len().to_string());
        key.push('\0');
        key.push_str(plugin);
        key.push('\0');
    }
    key.push('\0');
    for pattern in effective_ignore_patterns(agent) {
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
    key
}

pub(crate) struct CheckRuntime<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    session_roots: CheckSessionRoots<'a>,
}

enum CheckSessionRoots<'a> {
    #[cfg(test)]
    Fixed(&'a Path),
    Materialized(&'a StagedWorktreeView),
}

impl<'a> CheckRuntime<'a> {
    #[cfg(test)]
    pub(crate) fn fixed(
        root: &'a Path,
        snapshot_root: &'a Path,
        config: &'a CheckConfig,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            session_roots: CheckSessionRoots::Fixed(snapshot_root),
        }
    }

    pub(crate) fn materialized(
        root: &'a Path,
        staged_view: &'a StagedWorktreeView,
        config: &'a CheckConfig,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            session_roots: CheckSessionRoots::Materialized(staged_view),
        }
    }

    pub(crate) fn session_root_for_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<PathBuf, String> {
        match self.session_roots {
            #[cfg(test)]
            CheckSessionRoots::Fixed(path) => Ok(path.to_path_buf()),
            CheckSessionRoots::Materialized(staged_view) => {
                // `materialize_scope` receives both the q-scope/full-scope
                // base and the agent so it can apply configured ignore
                // patterns last when building the evaluator visible tree.
                staged_view.materialize_scope(agent, scope)
            }
        }
    }
}

pub(crate) struct InterrogationRunState {
    // This is a run-level pool of evaluator threads, not one thread. The
    // reuse key starts with evaluator model and visibleTreeOid, so a changed
    // visible tree cannot look up an existing session from another tree.
    pub(crate) thread_sessions_by_reuse_key: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) unavailable_models: BTreeSet<String>,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
}

impl InterrogationRunState {
    pub(crate) fn new() -> InterrogationRunState {
        InterrogationRunState {
            thread_sessions_by_reuse_key: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            unavailable_models: BTreeSet::new(),
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
        }
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
        self.thread_sessions_by_reuse_key.clear();
        self.session_instructions.clear();
    }
}
