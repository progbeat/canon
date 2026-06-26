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
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const IN_PLACE_VISIBLE_TREE_OID: &str = "in-place";
const IN_PLACE_VISIBLE_FILE_COUNT: usize = 1;

pub(crate) fn should_retry_full_scope_after_error(error: Option<&str>, scope: &[String]) -> bool {
    // This is the Interrogation Policy retry predicate only. The check-run
    // follow-up is executed in `src/check/run/execute/expectation.rs`, and
    // query-mode applies the same predicate in
    // `src/check/interrogation/query/mod.rs`.
    if scope == full_scope() {
        return false;
    }
    if error == Some(ERROR_SCOPE_TOO_NARROW) {
        return true;
    }
    false
}

pub(crate) struct EvaluatorThreadReuseKeyContext<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) visible_tree_oid: &'a str,
    pub(crate) turn_prompt: &'a str,
    pub(crate) question_context: &'a str,
    pub(crate) diff_base_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
}

pub(crate) fn evaluator_thread_reuse_key(
    context: EvaluatorThreadReuseKeyContext<'_>,
) -> Result<String, String> {
    // Evaluator thread reuse is context reuse, not a deterministic result cache.
    // A reused thread keeps its original developer instructions and live
    // thread-start context, so the key includes the model, the exact turn
    // prompt, the inputs that render the current developer-instructions
    // template, and the non-rendered context that changes the evaluator cwd or
    // tools. The turn prompt is part of the key because a live Codex thread is
    // conversational state, and each reused thread must stay tied to the same
    // evaluator task input. This protects answer correctness; started
    // expectation report liveness is owned by the check-run output layer.
    let mut key = String::new();
    app_server_model_key(context.model).push_cache_key_part(&mut key);
    key.push('\0');
    key.push_str(context.visible_tree_oid);
    key.push('\0');
    key.push_str(context.diff_base_tree_oid);
    key.push('\0');
    key.push_str(context.checked_tree_oid);
    key.push('\0');
    key.push_str(&context.turn_prompt.len().to_string());
    key.push('\0');
    key.push_str(context.turn_prompt);
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
        // This runtime mode owns the in-place evaluator view: no materialized
        // Git tree, full-project visible scope, stable fake visible-tree
        // metadata, and sessions rooted at the checked directory. Command
        // execution owns the complementary in-place rules: expectation
        // validation in `src/check/command/execution/in_place.rs`, cache-free
        // run orchestration in `src/check/command/execution/run.rs`, and
        // config expansion through `src/repo_inspection/mod.rs`.
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
            CheckRuntimeMode::Materialized { .. } => Some(self.root),
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
            CheckRuntimeMode::Materialized { .. } => None,
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
    // reuse key enforces the glossary's model/rendered-developer-instructions
    // invariant and also splits on stricter live thread-start context.
    pub(crate) thread_sessions_by_reuse_key: BTreeMap<String, String>,
    pub(crate) session_base_instructions: BTreeMap<String, String>,
    pub(crate) session_instructions: BTreeMap<String, String>,
    pub(crate) session_roots_by_id: BTreeMap<String, PathBuf>,
    pub(crate) visible_tree_oid_cache: VisibleTreeOidCache,
    pub(crate) parse_cache: EvaluatorResponseParseCache,
    pub(crate) prompt_template_output_dir_cache: PromptTemplateOutputDirCache,
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
            session_base_instructions: BTreeMap::new(),
            session_instructions: BTreeMap::new(),
            session_roots_by_id: BTreeMap::new(),
            visible_tree_oid_cache: VisibleTreeOidCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
            prompt_template_output_dir_cache: PromptTemplateOutputDirCache::new(),
            isolation_policy,
        })
    }

    pub(crate) fn models_in_retry_order(&self, agent: &AgentConfig) -> Vec<Option<String>> {
        evaluator_models(agent)
    }

    pub(crate) fn clear_thread_sessions(&mut self) {
        self.session_isolations.clear();
        self.thread_sessions_by_reuse_key.clear();
        self.session_base_instructions.clear();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_reuse_key_includes_developer_instruction_tree_inputs() {
        let agent = AgentConfig::default();
        let scope = full_scope();
        let base = evaluator_thread_reuse_key(EvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            visible_tree_oid: "visible-tree",
            turn_prompt: "Does it pass?",
            question_context: "instructions",
            diff_base_tree_oid: "base-a",
            checked_tree_oid: "checked-a",
        })
        .unwrap();
        let different_base = evaluator_thread_reuse_key(EvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            visible_tree_oid: "visible-tree",
            turn_prompt: "Does it pass?",
            question_context: "instructions",
            diff_base_tree_oid: "base-b",
            checked_tree_oid: "checked-a",
        })
        .unwrap();
        let different_checked = evaluator_thread_reuse_key(EvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            visible_tree_oid: "visible-tree",
            turn_prompt: "Does it pass?",
            question_context: "instructions",
            diff_base_tree_oid: "base-a",
            checked_tree_oid: "checked-b",
        })
        .unwrap();

        assert_ne!(base, different_base);
        assert_ne!(base, different_checked);
    }

    #[test]
    fn thread_reuse_key_includes_turn_prompt() {
        let agent = AgentConfig::default();
        let scope = full_scope();
        let first = evaluator_thread_reuse_key(EvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            visible_tree_oid: "visible-tree",
            turn_prompt: "Does alpha pass?",
            question_context: "",
            diff_base_tree_oid: "base",
            checked_tree_oid: "checked",
        })
        .unwrap();
        let second = evaluator_thread_reuse_key(EvaluatorThreadReuseKeyContext {
            agent: &agent,
            scope: &scope,
            model: Some("model"),
            visible_tree_oid: "visible-tree",
            turn_prompt: "Does beta pass?",
            question_context: "",
            diff_base_tree_oid: "base",
            checked_tree_oid: "checked",
        })
        .unwrap();

        assert_ne!(first, second);
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
    fn in_place_runtime_never_makes_requested_scope_smaller() {
        let root = PathBuf::from("/tmp/canon-in-place-runtime");
        let config = CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: AgentConfig::default(),
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
