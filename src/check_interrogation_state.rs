use crate::check_types::{CheckRecord, ObservedAnswerState};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator_config::app_server_model_key;
use crate::evaluator_response_cache::EvaluatorResponseParseCache;
use crate::evaluator_turn::evaluator_models;
use crate::hash::full_scope;
use crate::scope_hash::ScopeHashCache;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) fn should_retry_full_scope_after_restricted_idk(
    record: &CheckRecord,
    scope: &[String],
) -> bool {
    scope != full_scope()
        && ObservedAnswerState::from_observed(&record.observed) == ObservedAnswerState::Idk
}

pub(crate) fn evaluator_session_key(scope: &[String]) -> String {
    let mut key = String::new();
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
    pub(crate) snapshot_root: &'a Path,
    pub(crate) config: &'a CheckConfig,
}

pub(crate) struct InterrogationState {
    pub(crate) sessions_by_scope: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) unavailable_models: BTreeSet<String>,
    pub(crate) scope_hash_cache: ScopeHashCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
}

impl InterrogationState {
    pub(crate) fn new() -> InterrogationState {
        InterrogationState {
            sessions_by_scope: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            unavailable_models: BTreeSet::new(),
            scope_hash_cache: ScopeHashCache::new(),
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
        self.sessions_by_scope.clear();
        self.session_instructions.clear();
    }
}
