use crate::check::types::{CheckRecord, ObservedAnswerState, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator::config::app_server_model_key;
use crate::evaluator::response_cache::EvaluatorResponseParseCache;
use crate::evaluator::turn::evaluator_models;
use crate::git::tree_source::TreeSource;
use crate::git::visible_tree_oid::VisibleTreeOidCache;
use crate::hash::full_scope;
use crate::history::reuse::latest_stored_q_scope_with_cache;
use crate::history::HistoryCache;
use crate::isolation::{NaiveIsolationGuard, NaiveIsolationPolicy};
use crate::scope::effective_ignore_patterns;
use crate::staged::StagedWorktreeView;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[cfg(test)]
static STAGED_RUNTIME_TREE_SOURCE: TreeSource = TreeSource::Staged;

pub(crate) fn should_retry_full_scope_after_restricted_response(
    record: &CheckRecord,
    scope: &[String],
) -> bool {
    if scope == full_scope() {
        return false;
    }
    if ObservedAnswerState::from_error(record.error.as_deref())
        == ObservedAnswerState::InsufficientEvidence
    {
        return true;
    }
    false
}

pub(crate) fn initial_visible_scope_for_expectation(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scheduled_full_scope_reset_ids: &BTreeSet<String>,
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
    if scheduled_full_scope_reset_ids.contains(&expectation.id) {
        return Ok(full_scope());
    }
    Ok(
        latest_stored_q_scope_with_cache(root, &expectation.agent, expectation, history_cache)?
            .unwrap_or_else(full_scope),
    )
}

pub(crate) fn evaluator_thread_reuse_key(
    agent: &AgentConfig,
    scope: &[String],
    model: Option<&str>,
    visible_tree_oid: &str,
) -> String {
    // The glossary's thread invariant is one-way: a reused thread must keep
    // the same evaluator model and visible tree, and different model/tree
    // inputs must not share a thread. Extra key parts below are stricter
    // developer-instruction inputs that prevent unsafe reuse without allowing
    // cross-model or cross-visible-tree reuse.
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
    pub(crate) tree_source: &'a TreeSource,
    no_sandbox: bool,
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
            tree_source: &STAGED_RUNTIME_TREE_SOURCE,
            no_sandbox: true,
            session_roots: CheckSessionRoots::Fixed(snapshot_root),
        }
    }

    pub(crate) fn materialized(
        root: &'a Path,
        staged_view: &'a StagedWorktreeView,
        tree_source: &'a TreeSource,
        config: &'a CheckConfig,
        no_sandbox: bool,
    ) -> CheckRuntime<'a> {
        CheckRuntime {
            root,
            config,
            tree_source,
            no_sandbox,
            session_roots: CheckSessionRoots::Materialized(staged_view),
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
        match self.session_roots {
            #[cfg(test)]
            CheckSessionRoots::Fixed(path) => Ok(path.to_path_buf()),
            CheckSessionRoots::Materialized(staged_view) => {
                // Configured ignore patterns shape the evaluator-visible Git
                // tree before the lazy hardlink materialization step.
                staged_view.materialize_evaluator_scope(agent, scope, visible_tree_oid)
            }
        }
    }
}

pub(crate) struct InterrogationRunState {
    pub(crate) session_isolations: BTreeMap<String, NaiveIsolationGuard>,
    // This is a run-level pool of evaluator threads, not one thread. The
    // reuse key starts with evaluator model and visibleTreeOid, so a changed
    // visible tree cannot look up an existing session from another tree.
    pub(crate) thread_sessions_by_reuse_key: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) unavailable_models: BTreeSet<String>,
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
