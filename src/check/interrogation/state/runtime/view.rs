use super::{CheckRuntime, CheckRuntimeMode};
use crate::config_types::AgentConfig;
use crate::git::VisibleTreeOidCache;
use crate::hash::full_scope;
use crate::scope::visible_scope;
use std::path::PathBuf;

impl CheckRuntime<'_> {
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
            // In-place q-scopes are normalized to full project scope. Config
            // validation rejects configured `q-scope` in this mode, while
            // last-pass q-scopes and evaluator qScopeSuggestion values cannot
            // hide files.
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
            // In-place evaluator threads start in the checked directory
            // itself; no scoped materialization is created.
            return Ok(self.root.to_path_buf());
        }
        // `visible_scope` returns the complete visible-scope pathspec,
        // including configured ignore exclusions. From here down,
        // materialization selects paths solely by applying that pathspec to
        // checked Git entries.
        let visible_scope = visible_scope(agent, scope)?;
        match &self.mode {
            CheckRuntimeMode::Materialized {
                tree_materializer, ..
            } => {
                let visible_tree_oid = visible_tree_oid
                    .ok_or("materialized evaluator view is missing its visible tree OID")?;
                tree_materializer.materialize_visible_scope(&visible_scope, visible_tree_oid)
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
            CheckRuntimeMode::Materialized {
                tree_materializer, ..
            } => Ok(tree_materializer.visible_tree_root_path(visible_tree_oid)),
            CheckRuntimeMode::InPlaceCheck { .. } | CheckRuntimeMode::InPlaceTemporaryQuery => {
                Err("in-place evaluator view has no materialized session root".to_string())
            }
        }
    }
}
