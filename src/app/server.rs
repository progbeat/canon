use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config_types::AgentConfig;
use crate::evaluator::{
    is_model_technical_failure, EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorRunner,
};
use crate::state_paths::canon_state_path;
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};

use super::process::AppServerRunner;

pub(crate) struct LazyAppServerRunner {
    app_server_root: PathBuf,
    app_server_state_root: Option<PathBuf>,
    load_plugins: bool,
    agent: AgentConfig,
    no_sandbox: bool,
    inner: Option<AppServerRunner>,
    sessions: BTreeSet<String>,
    retired_token_usage: TokenUsage,
}

impl LazyAppServerRunner {
    pub(crate) fn new(
        app_server_root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> Result<LazyAppServerRunner, String> {
        let app_server_state_root = canon_state_path(app_server_root, "")?;
        Ok(LazyAppServerRunner::with_state_root(
            app_server_root,
            Some(app_server_state_root),
            load_plugins,
            agent,
            no_sandbox,
        ))
    }

    pub(crate) fn new_in_place(
        app_server_root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> Result<LazyAppServerRunner, String> {
        Ok(LazyAppServerRunner::new_without_state(
            app_server_root,
            load_plugins,
            agent,
            no_sandbox,
        ))
    }

    pub(crate) fn new_without_state(
        app_server_root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> LazyAppServerRunner {
        LazyAppServerRunner::with_state_root(app_server_root, None, load_plugins, agent, no_sandbox)
    }

    fn with_state_root(
        app_server_root: &std::path::Path,
        app_server_state_root: Option<PathBuf>,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> LazyAppServerRunner {
        LazyAppServerRunner {
            app_server_root: app_server_root.to_path_buf(),
            app_server_state_root,
            load_plugins,
            agent: agent.clone(),
            no_sandbox,
            inner: None,
            sessions: BTreeSet::new(),
            retired_token_usage: TokenUsage::default(),
        }
    }

    fn inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(
                &self.app_server_root,
                self.app_server_state_root.as_deref(),
                self.load_plugins,
                &self.agent,
                self.no_sandbox,
            )?);
        }
        match self.inner.as_mut() {
            Some(inner) => Ok(inner),
            None => Err("app-server runner is not initialized".into()),
        }
    }

    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        let mut total = self.retired_token_usage;
        if let Some(usage) = self.inner.as_ref().and_then(AppServerRunner::token_usage) {
            total = total.add(usage);
        }
        if total.total_tokens == 0 {
            None
        } else {
            Some(total)
        }
    }

    pub(crate) fn drain_token_usage_updates(&mut self) -> Result<(), EvaluatorError> {
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates()?;
        }
        Ok(())
    }

    fn retire_inner_after_model_failure(
        &mut self,
        err: &EvaluatorError,
    ) -> Result<(), EvaluatorError> {
        if !is_model_technical_failure(err) {
            return Ok(());
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates()?;
            if let Some(usage) = inner.token_usage() {
                self.retired_token_usage = self.retired_token_usage.add(usage);
            }
        }
        self.sessions.clear();
        self.inner = None;
        Ok(())
    }
}

impl EvaluatorRunner for LazyAppServerRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        template_artifact_paths: &[PathBuf],
        base_instructions: &str,
        developer_instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
        dynamic_tools: &[serde_json::Value],
    ) -> Result<String, EvaluatorError> {
        let result = self.inner()?.start_session(
            session_cwd,
            template_artifact_paths,
            base_instructions,
            developer_instructions,
            agent,
            model,
            thinking,
            scope,
            dynamic_tools,
        );
        match result {
            Ok(session_id) => {
                self.sessions.insert(session_id.clone());
                Ok(session_id)
            }
            Err(err) => {
                self.retire_inner_after_model_failure(&err)?;
                Err(err)
            }
        }
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
        output_schema: &serde_json::Value,
        dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
    ) -> Result<String, EvaluatorError> {
        if !self.sessions.contains(session_id) {
            return Err("app-server runner does not own session".into());
        }
        let result = self
            .inner
            .as_mut()
            .ok_or_else(|| EvaluatorError::message("app-server runner is not initialized"))?
            .ask(
                session_id,
                prompt,
                model,
                thinking,
                output_schema,
                dynamic_tool_handler,
            );
        if let Err(err) = &result {
            self.retire_inner_after_model_failure(err)?;
        }
        result
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.inner
            .as_mut()
            .and_then(AppServerRunner::take_last_turn_usage)
    }

    fn take_retired_sessions(&mut self) -> Vec<String> {
        let Some(inner) = self.inner.as_mut() else {
            return Vec::new();
        };
        let retired = inner.drain_retired_sessions();
        for session_id in &retired {
            self.sessions.remove(session_id);
        }
        retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use std::fs;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn in_place_runner_initializes_outside_git_worktree() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "canon-in-place-runner-no-git-{}-{}",
            process::id(),
            stamp
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        LazyAppServerRunner::new_in_place(&root, false, &AgentConfig::default(), false).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runner_without_state_root_omits_persistent_state() {
        let root = std::env::temp_dir();
        let runner =
            LazyAppServerRunner::new_without_state(&root, false, &AgentConfig::default(), false);

        assert!(runner.app_server_state_root.is_none());
    }
}
