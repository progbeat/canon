use crate::evaluator::{
    AppServerArgs, EvaluatorProcessIsolation, EvaluatorProgress, EvaluatorProjectFilesystem,
};
use crate::token_usage::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use super::child::EvaluatorCodexHome;

pub(super) struct AppServerRunner {
    // Drop releases these runtime inputs explicitly after terminating and
    // reaping the child and joining both output readers.
    pub(super) evaluator_runtime_inputs: Option<EvaluatorCodexHome>,
    pub(super) child: Child,
    pub(super) stdin: ChildStdin,
    pub(super) messages: Receiver<Result<Value, String>>,
    pub(super) reader: Option<JoinHandle<()>>,
    pub(super) stderr: Receiver<Vec<u8>>,
    pub(super) stderr_reader: Option<JoinHandle<()>>,
    pub(super) next_id: u64,
    pub(super) token_usage_by_turn: BTreeMap<String, TokenUsage>,
    // Last cumulative app-server snapshot for each thread.
    pub(super) latest_token_usage_by_thread: BTreeMap<String, TokenUsage>,
    pub(super) token_usage_updates_by_turn: BTreeMap<String, Vec<TokenUsageUpdate>>,
    pub(super) context_compaction_events_by_turn: BTreeMap<String, Vec<ContextCompactionEvent>>,
    pub(super) last_turn_usage: Option<EvaluatorTurnUsage>,
    pub(super) retired_threads: BTreeSet<String>,
    pub(super) thread_runtime_inputs: BTreeMap<String, ThreadRuntimeInputs>,
    pub(super) progress: Option<EvaluatorProgress>,
    pub(super) process_isolation: EvaluatorProcessIsolation,
    pub(super) project_filesystem: EvaluatorProjectFilesystem,
    pub(super) startup_args: Option<AppServerArgs>,
    pub(super) codex_executable: PathBuf,
}

#[derive(Clone)]
pub(super) struct ThreadRuntimeInputs {
    pub(super) cwd: PathBuf,
    pub(super) template_artifact_directory: PathBuf,
    pub(super) project_tools_advertised: bool,
}

impl AppServerRunner {
    pub(super) fn process_isolation(&self) -> EvaluatorProcessIsolation {
        self.process_isolation
    }

    pub(super) fn thread_cwd(&self, thread_id: &str) -> Option<&Path> {
        self.thread_runtime_inputs
            .get(thread_id)
            .map(|inputs| inputs.cwd.as_path())
    }

    pub(super) fn thread_runtime_inputs(&self, thread_id: &str) -> Option<&ThreadRuntimeInputs> {
        self.thread_runtime_inputs.get(thread_id)
    }

    pub(super) fn codex_executable(&self) -> &Path {
        &self.codex_executable
    }

    pub(super) fn remember_thread_runtime_inputs(
        &mut self,
        thread_id: String,
        cwd: PathBuf,
        template_artifact_directory: PathBuf,
        project_tools_advertised: bool,
    ) {
        self.thread_runtime_inputs.insert(
            thread_id,
            ThreadRuntimeInputs {
                cwd,
                template_artifact_directory,
                project_tools_advertised,
            },
        );
    }

    pub(super) fn take_last_turn_usage_record(&mut self) -> Option<EvaluatorTurnUsage> {
        self.last_turn_usage.take()
    }

    pub(super) fn drain_retired_threads(&mut self) -> Vec<String> {
        let retired = std::mem::take(&mut self.retired_threads)
            .into_iter()
            .collect::<Vec<_>>();
        for thread_id in &retired {
            self.thread_runtime_inputs.remove(thread_id);
        }
        retired
    }

    pub(super) fn set_progress_reporter(&mut self, progress: Option<EvaluatorProgress>) {
        self.progress = progress;
    }

    pub(super) fn record_turn_message_activity_progress(&self) {
        if let Some(progress) = &self.progress {
            progress.record_turn_message_activity();
        }
    }

    pub(super) fn record_turn_timeout_progress(&self) {
        if let Some(progress) = &self.progress {
            progress.record_turn_timeout();
        }
    }
}
