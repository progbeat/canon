use crate::app::process::EvaluatorCodexHome;
use crate::evaluator::{AppServerArgs, EvaluatorProgress};
use crate::token_usage_types::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

pub(crate) struct AppServerRunner {
    pub(super) app_server_state_root: Option<PathBuf>,
    // Ownership keeps the isolated home alive through child termination and
    // removes it when the runner is dropped.
    pub(super) _evaluator_codex_home: EvaluatorCodexHome,
    pub(super) child: Child,
    pub(super) stdin: ChildStdin,
    pub(super) messages: Receiver<Result<Value, String>>,
    pub(super) reader: Option<JoinHandle<()>>,
    pub(super) stderr: Receiver<String>,
    pub(super) stderr_reader: Option<JoinHandle<()>>,
    pub(super) next_id: u64,
    pub(super) token_usage_by_turn: BTreeMap<String, TokenUsage>,
    pub(super) token_usage_updates_by_turn: BTreeMap<String, Vec<TokenUsageUpdate>>,
    pub(super) context_compaction_events_by_turn: BTreeMap<String, Vec<ContextCompactionEvent>>,
    pub(super) last_turn_usage: Option<EvaluatorTurnUsage>,
    pub(super) retired_sessions: BTreeSet<String>,
    pub(super) session_cwds: BTreeMap<String, PathBuf>,
    pub(super) progress: Option<EvaluatorProgress>,
    pub(super) no_sandbox: bool,
    pub(super) startup_args: Option<AppServerArgs>,
}

impl AppServerRunner {
    pub(crate) fn app_server_state_root(&self) -> Option<&Path> {
        self.app_server_state_root.as_deref()
    }

    pub(crate) fn no_sandbox(&self) -> bool {
        self.no_sandbox
    }

    pub(crate) fn session_cwd(&self, session_id: &str) -> Option<&Path> {
        self.session_cwds.get(session_id).map(PathBuf::as_path)
    }

    pub(crate) fn remember_session_cwd(&mut self, session_id: String, session_cwd: PathBuf) {
        self.session_cwds.insert(session_id, session_cwd);
    }

    pub(crate) fn take_last_turn_usage_record(&mut self) -> Option<EvaluatorTurnUsage> {
        self.last_turn_usage.take()
    }

    pub(crate) fn drain_retired_sessions(&mut self) -> Vec<String> {
        let retired = std::mem::take(&mut self.retired_sessions)
            .into_iter()
            .collect::<Vec<_>>();
        for session_id in &retired {
            self.session_cwds.remove(session_id);
        }
        retired
    }

    pub(crate) fn set_progress_reporter(&mut self, progress: Option<EvaluatorProgress>) {
        self.progress = progress;
    }

    pub(crate) fn record_turn_message_activity_progress(&self) {
        if let Some(progress) = &self.progress {
            progress.record_turn_message_activity();
        }
    }

    pub(crate) fn record_turn_timeout_progress(&self) {
        if let Some(progress) = &self.progress {
            progress.record_turn_timeout();
        }
    }
}
