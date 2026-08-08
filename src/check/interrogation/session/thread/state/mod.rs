use crate::evaluator::{InvocationResponseParseMemo, PromptRenderer};
use crate::isolation::NaiveIsolationPolicy;
use crate::platform::filesystem::PrivateTemporaryDirectoryAllocator;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

mod materialization;
mod registry;
mod reuse_key;
mod workspace;

use materialization::MaterializedSessionRoot;
use registry::ThreadRegistry;
pub(in crate::check::interrogation::session::thread) use reuse_key::{
    prerender_evaluator_thread_reuse_key, rendered_evaluator_thread_reuse_key,
    PrerenderEvaluatorThreadReuseKey, PrerenderEvaluatorThreadReuseKeyContext,
    RenderedEvaluatorThreadReuseKey, RenderedEvaluatorThreadReuseKeyContext,
};
pub(in crate::check::interrogation::session::thread) use workspace::ThreadWorkspace;

pub(in crate::check::interrogation::session) struct ThreadState {
    // One run-level entry owns each distinct materialized root. Isolated
    // entries retain their restoration guard, and all evaluator threads for
    // the same visible tree share the cached session root until this state is
    // dropped.
    materialized_session_roots: BTreeMap<PathBuf, MaterializedSessionRoot>,
    thread_registry: ThreadRegistry,
    invocation_response_parse_memo: InvocationResponseParseMemo,
    prompt_renderer: Arc<PromptRenderer>,
    isolation_policy: Option<NaiveIsolationPolicy>,
}

impl ThreadState {
    pub(in crate::check::interrogation::session) fn new(
        disable_session_isolation: bool,
        temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
    ) -> Result<ThreadState, String> {
        let isolation_policy = if disable_session_isolation {
            None
        } else {
            Some(NaiveIsolationPolicy::from_env()?)
        };
        Ok(ThreadState {
            materialized_session_roots: BTreeMap::new(),
            thread_registry: ThreadRegistry::default(),
            invocation_response_parse_memo: InvocationResponseParseMemo::new(),
            prompt_renderer: Arc::new(PromptRenderer::new(temporary_directory_allocator)),
            isolation_policy,
        })
    }

    pub(super) fn thread_registry(&self) -> &ThreadRegistry {
        &self.thread_registry
    }

    pub(super) fn thread_registry_mut(&mut self) -> &mut ThreadRegistry {
        &mut self.thread_registry
    }

    pub(in crate::check::interrogation::session) fn clear_threads(&mut self) {
        self.thread_registry.clear_threads();
    }

    pub(in crate::check::interrogation::session) fn discard_thread(&mut self, thread_id: &str) {
        self.thread_registry.discard_thread(thread_id);
    }

    pub(super) fn response_parse_memo_mut(&mut self) -> &mut InvocationResponseParseMemo {
        &mut self.invocation_response_parse_memo
    }

    pub(super) fn prompt_renderer(&self) -> Arc<PromptRenderer> {
        Arc::clone(&self.prompt_renderer)
    }
}
