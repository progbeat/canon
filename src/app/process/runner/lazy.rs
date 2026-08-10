use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config_types::AgentConfig;
use crate::evaluator::{
    is_technical_failure, EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorHostIsolation,
    EvaluatorProcessIsolation, EvaluatorProgress, EvaluatorProjectFilesystem, EvaluatorRunner,
    ReadOnlyProjectInspectionPlan,
};
use crate::state_paths::canon_state_path;
use crate::token_usage::{EvaluatorTurnUsage, TokenUsage};

use super::evaluator::{AppServerThreadStartContext, InvocationThreadStartMemo};
use super::AppServerRunner;

pub(crate) struct LazyAppServerRunner {
    app_server_state_root: Option<PathBuf>,
    host_isolation: EvaluatorHostIsolation,
    load_plugins: bool,
    agent: AgentConfig,
    inspection_plan: ReadOnlyProjectInspectionPlan,
    project_filesystem: EvaluatorProjectFilesystem,
    inner: Option<AppServerRunner>,
    progress: Option<EvaluatorProgress>,
    threads: BTreeSet<String>,
    retired_threads: BTreeSet<String>,
    last_turn_usage: Option<EvaluatorTurnUsage>,
    retired_token_usage: TokenUsage,
    invocation_thread_start_memo: InvocationThreadStartMemo,
}

impl LazyAppServerRunner {
    pub(crate) fn new(
        project_root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
        process_isolation: EvaluatorProcessIsolation,
    ) -> Result<LazyAppServerRunner, String> {
        let app_server_state_root = canon_state_path(project_root, "")?;
        let host_isolation = EvaluatorHostIsolation::for_project(project_root)?;
        LazyAppServerRunner::with_state_root(
            Some(app_server_state_root),
            host_isolation,
            load_plugins,
            agent,
            process_isolation,
            EvaluatorProjectFilesystem::ImmutableSnapshot,
        )
    }

    pub(crate) fn new_in_place(
        load_plugins: bool,
        agent: &AgentConfig,
        process_isolation: EvaluatorProcessIsolation,
    ) -> Result<LazyAppServerRunner, String> {
        LazyAppServerRunner::with_state_root(
            None,
            EvaluatorHostIsolation::in_place(),
            load_plugins,
            agent,
            process_isolation,
            // [l,90,KD] In-place inspection applies no q-scope, ignore, cached
            // scope, or retry filtering: every safely inspectable regular file
            // under the checked directory remains visible on every call. The
            // project tool's no-symlink and no-Git-administration rules are
            // capability boundaries, not path-hiding selection behavior.
            EvaluatorProjectFilesystem::LiveReadOnlyInspection,
        )
    }

    pub(crate) fn new_temporary_query(
        project_root: &Path,
        load_plugins: bool,
        agent: &AgentConfig,
        process_isolation: EvaluatorProcessIsolation,
    ) -> Result<LazyAppServerRunner, String> {
        LazyAppServerRunner::with_state_root(
            None,
            EvaluatorHostIsolation::for_project(project_root)?,
            load_plugins,
            agent,
            process_isolation,
            EvaluatorProjectFilesystem::ImmutableSnapshot,
        )
    }

    fn with_state_root(
        app_server_state_root: Option<PathBuf>,
        host_isolation: EvaluatorHostIsolation,
        load_plugins: bool,
        agent: &AgentConfig,
        process_isolation: EvaluatorProcessIsolation,
        project_filesystem: EvaluatorProjectFilesystem,
    ) -> Result<LazyAppServerRunner, String> {
        let inspection_plan =
            ReadOnlyProjectInspectionPlan::for_process_isolation(process_isolation)?;
        Ok(LazyAppServerRunner {
            app_server_state_root,
            host_isolation,
            load_plugins,
            agent: agent.clone(),
            inspection_plan,
            project_filesystem,
            inner: None,
            progress: None,
            threads: BTreeSet::new(),
            retired_threads: BTreeSet::new(),
            last_turn_usage: None,
            retired_token_usage: TokenUsage::default(),
            invocation_thread_start_memo: InvocationThreadStartMemo::default(),
        })
    }

    fn inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
        if self.inner.is_none() {
            let mut inner = AppServerRunner::new(
                self.load_plugins,
                &self.agent,
                &self.inspection_plan,
                self.project_filesystem,
            )?;
            inner.set_progress_reporter(self.progress.clone());
            self.inner = Some(inner);
        }
        self.active_inner()
    }

    fn active_inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
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
        if !is_technical_failure(err) {
            return Ok(());
        }
        let drain_result = if let Some(inner) = self.inner.as_mut() {
            let drain_result = inner.drain_token_usage_updates();
            if let Some(usage) = inner.token_usage() {
                self.retired_token_usage = self.retired_token_usage.add(usage);
            }
            drain_result
        } else {
            Ok(())
        };
        // [fD,kg] Retiring the app-server process invalidates every thread it
        // owned, not only the thread whose turn exposed the technical failure.
        // Preserve those IDs until the interrogation registry consumes them;
        // dropping them here would leave stale threads eligible for reuse.
        self.retired_threads.append(&mut self.threads);
        self.inner = None;
        drain_result
    }

    fn finish_with_model_failure_retirement<T>(
        &mut self,
        result: Result<T, EvaluatorError>,
    ) -> Result<T, EvaluatorError> {
        match result {
            Ok(value) => Ok(value),
            Err(mut err) => {
                if let Err(retire_error) = self.retire_inner_after_model_failure(&err) {
                    err = err.with_appended_message(format!(
                        "evaluator runner was retired, but draining its remaining usage failed: \
                         {retire_error}"
                    ));
                }
                Err(err)
            }
        }
    }
}

impl EvaluatorRunner for LazyAppServerRunner {
    fn evaluator_dynamic_tools(&self) -> Result<Vec<serde_json::Value>, EvaluatorError> {
        Ok(self.inspection_plan.dynamic_tools().to_vec())
    }

    fn start_thread(
        &mut self,
        thread_cwd: &Path,
        template_artifact_directory: &Path,
        rendered_base_text: &str,
        rendered_developer_text: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        dynamic_tools: &[serde_json::Value],
    ) -> Result<String, EvaluatorError> {
        let codex_executable = self.inner()?.codex_executable().to_path_buf();
        let params = self
            .invocation_thread_start_memo
            .resolve(AppServerThreadStartContext {
                cwd: thread_cwd,
                template_artifact_directory,
                host_isolation: &self.host_isolation,
                rendered_base_text,
                rendered_developer_text,
                agent,
                model,
                thinking,
                app_server_state_root: self.app_server_state_root.as_deref(),
                process_isolation: self.inspection_plan.process_isolation(),
                dynamic_tools,
                codex_executable: &codex_executable,
            })?;
        let project_tools_advertised =
            ReadOnlyProjectInspectionPlan::advertises_project_tools(dynamic_tools);
        // xpec: bP,KD,hQ
        assert!(
            project_tools_advertised,
            "thread dynamic tools must preserve the read-only project inspection plan"
        );
        let result = self.active_inner()?.start_thread(
            thread_cwd,
            template_artifact_directory,
            project_tools_advertised,
            params,
        );
        let thread_id = self.finish_with_model_failure_retirement(result)?;
        self.retired_threads.remove(&thread_id);
        self.threads.insert(thread_id.clone());
        Ok(thread_id)
    }

    fn ask(
        &mut self,
        thread_id: &str,
        rendered_turn_text: &str,
        model: Option<&str>,
        thinking: &str,
        output_schema: &serde_json::Value,
        dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
    ) -> Result<String, EvaluatorError> {
        self.last_turn_usage = None;
        if !self.threads.contains(thread_id) {
            return Err("app-server runner does not own thread".into());
        }
        let (result, last_turn_usage) = {
            let inner = self.active_inner()?;
            let result = inner.ask(
                thread_id,
                rendered_turn_text,
                model,
                thinking,
                output_schema,
                dynamic_tool_handler,
            );
            let last_turn_usage = inner.take_last_turn_usage();
            (result, last_turn_usage)
        };
        // [kK] Move per-turn telemetry across the lazy boundary before a
        // technical failure can retire and drop the inner app-server runner.
        self.last_turn_usage = last_turn_usage;
        self.finish_with_model_failure_retirement(result)
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.last_turn_usage.take()
    }

    fn take_retired_threads(&mut self) -> Vec<String> {
        let mut retired = std::mem::take(&mut self.retired_threads);
        if let Some(inner) = self.inner.as_mut() {
            for thread_id in inner.drain_retired_threads() {
                self.threads.remove(&thread_id);
                retired.insert(thread_id);
            }
        }
        retired.into_iter().collect()
    }

    fn set_progress_reporter(&mut self, progress: Option<EvaluatorProgress>) {
        self.progress = progress.clone();
        if let Some(inner) = self.inner.as_mut() {
            inner.set_progress_reporter(progress);
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use crate::evaluator::EvaluatorFailureKind;

    #[test] // xpec: fD,kg
    fn technical_failure_reports_every_runner_thread_as_retired() {
        let mut runner = LazyAppServerRunner::new_in_place(
            false,
            &AgentConfig::default(),
            EvaluatorProcessIsolation::CanonManaged,
        )
        .unwrap();
        runner
            .threads
            .extend(["thread-a".to_string(), "thread-b".to_string()]);
        let error = EvaluatorError::failure(EvaluatorFailureKind::TurnTimeout, "turn timed out");

        runner.retire_inner_after_model_failure(&error).unwrap();

        assert!(runner.threads.is_empty());
        assert_eq!(
            runner.take_retired_threads(),
            vec!["thread-a".to_string(), "thread-b".to_string()]
        );
        assert!(runner.take_retired_threads().is_empty());
    }
}
