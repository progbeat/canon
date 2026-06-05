use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use serde_json::Value;

use crate::config_types::AgentConfig;
use crate::evaluator::{EvaluatorError, ModelCatalogFile};
use crate::token_usage_types::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};

pub(crate) struct AppServerRunner {
    pub(crate) app_server_root: PathBuf,
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) messages: Receiver<Result<Value, String>>,
    pub(crate) reader: Option<JoinHandle<()>>,
    pub(crate) stderr: Receiver<String>,
    pub(crate) stderr_reader: Option<JoinHandle<()>>,
    pub(crate) next_id: u64,
    pub(crate) token_usage_by_turn: BTreeMap<String, TokenUsage>,
    pub(crate) token_usage_updates_by_turn: BTreeMap<String, Vec<TokenUsageUpdate>>,
    pub(crate) context_compaction_events_by_turn: BTreeMap<String, Vec<ContextCompactionEvent>>,
    pub(crate) last_turn_usage: Option<EvaluatorTurnUsage>,
    pub(crate) retired_sessions: BTreeSet<String>,
    pub(crate) session_cwds: BTreeMap<String, PathBuf>,
    pub(crate) no_sandbox: bool,
    pub(crate) startup_model_catalog_file: Option<ModelCatalogFile>,
}

pub(crate) struct LazyAppServerRunner {
    pub(crate) app_server_root: PathBuf,
    pub(crate) load_plugins: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) no_sandbox: bool,
    pub(crate) inner: Option<AppServerRunner>,
    pub(crate) sessions: BTreeSet<String>,
    pub(crate) retired_token_usage: TokenUsage,
}

impl LazyAppServerRunner {
    pub(crate) fn new(
        app_server_root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> LazyAppServerRunner {
        LazyAppServerRunner {
            app_server_root: app_server_root.to_path_buf(),
            load_plugins,
            agent: agent.clone(),
            no_sandbox,
            inner: None,
            sessions: BTreeSet::new(),
            retired_token_usage: TokenUsage::default(),
        }
    }

    pub(crate) fn inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(
                &self.app_server_root,
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
}

impl AppServerRunner {
    pub(crate) fn drain_retired_sessions(&mut self) -> Vec<String> {
        let retired = std::mem::take(&mut self.retired_sessions)
            .into_iter()
            .collect::<Vec<_>>();
        for session_id in &retired {
            self.session_cwds.remove(session_id);
        }
        retired
    }
}
