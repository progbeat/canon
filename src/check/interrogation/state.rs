use crate::check::core::ERROR_SCOPE_TOO_NARROW;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::evaluator::{app_server_model_key, evaluator_models, EvaluatorResponseParseCache};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::isolation::{NaiveIsolationGuard, NaiveIsolationPolicy};
use crate::scope::{effective_ignore_patterns, visible_scope};
use crate::staged::StagedWorktreeView;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const IN_PLACE_VISIBLE_TREE_OID: &str = "in-place";

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
    diff_base_tree_oid: &str,
    checked_tree_oid: &str,
) -> Result<String, String> {
    // Evaluator thread reuse is context reuse, not a deterministic result cache.
    // A reused thread keeps its original developer instructions and live
    // thread-start context, so the key includes the model, the inputs that
    // render the current developer-instructions template, and the non-rendered
    // context that changes the evaluator cwd or tools.
    let mut key = String::new();
    app_server_model_key(model).push_cache_key_part(&mut key);
    key.push('\0');
    key.push_str(visible_tree_oid);
    key.push('\0');
    key.push_str(diff_base_tree_oid);
    key.push('\0');
    key.push_str(checked_tree_oid);
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
    no_sandbox: bool,
    mode: CheckRuntimeMode<'a>,
}

enum CheckRuntimeMode<'a> {
    Materialized {
        tree_source: &'a TreeSource,
        tree_context: CheckTreeContext,
        staged_view: &'a StagedWorktreeView,
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
            },
        }
    }

    pub(crate) fn in_place(
        root: &'a Path,
        config: &'a CheckConfig,
        no_sandbox: bool,
    ) -> CheckRuntime<'a> {
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
            CheckRuntimeMode::InPlace => 0,
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
            CheckRuntimeMode::InPlace => Ok(0),
        }
    }

    pub(crate) fn visible_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        if self.is_in_place() {
            // In-place mode has no Git-backed visible tree and no path hiding.
            // CLI parsing rejects `--scope` in this mode, while stored q-scopes
            // and narrowing retries are intentionally ignored.
            return Ok(full_scope());
        }
        visible_scope(agent, scope)
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
    // reuse key enforces the glossary's model/rendered-developer-instructions
    // invariant and also splits on stricter live thread-start context.
    pub(crate) thread_sessions_by_reuse_key: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) session_roots_by_id: BTreeMap<String, PathBuf>,
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
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
            isolation_policy,
        })
    }

    pub(crate) fn models_in_retry_order(&self, agent: &AgentConfig) -> Vec<Option<String>> {
        evaluator_models(agent)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_reuse_key_includes_developer_instruction_tree_inputs() {
        let agent = AgentConfig::default();
        let scope = full_scope();
        let base = evaluator_thread_reuse_key(
            &agent,
            &scope,
            Some("model"),
            "visible-tree",
            "instructions",
            "base-a",
            "checked-a",
        )
        .unwrap();
        let different_base = evaluator_thread_reuse_key(
            &agent,
            &scope,
            Some("model"),
            "visible-tree",
            "instructions",
            "base-b",
            "checked-a",
        )
        .unwrap();
        let different_checked = evaluator_thread_reuse_key(
            &agent,
            &scope,
            Some("model"),
            "visible-tree",
            "instructions",
            "base-a",
            "checked-b",
        )
        .unwrap();

        assert_ne!(base, different_base);
        assert_ne!(base, different_checked);
    }

    #[test]
    fn in_place_runtime_uses_full_scope_and_checked_directory() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: AgentConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, false);
        let requested_scope = vec!["src".to_string()];

        assert_eq!(
            runtime
                .visible_scope(&AgentConfig::default(), &requested_scope)
                .unwrap(),
            full_scope()
        );
        assert_eq!(
            runtime
                .session_root_for_scope(&AgentConfig::default(), &requested_scope, "in-place")
                .unwrap(),
            root
        );
    }
}
