use super::super::reuse_key::{PrerenderEvaluatorThreadReuseKey, RenderedEvaluatorThreadReuseKey};
use super::{RegisteredThread, StoredThreadInstructions, ThreadRegistry};
use std::collections::BTreeSet;

impl ThreadRegistry {
    pub(in crate::check::interrogation::session::thread) fn reusable_thread_by_prerender_reuse_key(
        &self,
        prerender_reuse_key: &PrerenderEvaluatorThreadReuseKey,
        expectation_id: Option<&str>,
    ) -> Option<String> {
        self.reusable_thread(
            self.threads_by_prerender_reuse_key
                .get(prerender_reuse_key)
                .cloned(),
            expectation_id,
        )
    }

    pub(in crate::check::interrogation::session::thread) fn reusable_thread_by_rendered_reuse_key(
        &self,
        rendered_reuse_key: &RenderedEvaluatorThreadReuseKey,
        expectation_id: Option<&str>,
    ) -> Option<String> {
        self.reusable_thread(
            self.threads_by_rendered_reuse_key
                .get(rendered_reuse_key)
                .cloned(),
            expectation_id,
        )
    }

    fn reusable_thread(
        &self,
        candidate_thread_id: Option<String>,
        expectation_id: Option<&str>,
    ) -> Option<String> {
        // [F] A thread that received `canon.show` output for an expectation
        // must not be reused to interrogate that expectation again.
        candidate_thread_id.filter(|thread_id| {
            self.threads.contains_key(thread_id)
                && !self.thread_has_seen_canon_show_expectation(thread_id, expectation_id)
        })
    }

    pub(in crate::check::interrogation::session::thread) fn bind_prerender_reuse_key_to_thread(
        &mut self,
        prerender_reuse_key: PrerenderEvaluatorThreadReuseKey,
        thread_id: String,
    ) {
        if self.threads.contains_key(&thread_id) {
            self.threads_by_prerender_reuse_key
                .insert(prerender_reuse_key, thread_id);
        }
    }

    pub(in crate::check::interrogation::session::thread) fn register_thread(
        &mut self,
        thread_id: String,
        prerender_reuse_key: PrerenderEvaluatorThreadReuseKey,
        rendered_reuse_key: RenderedEvaluatorThreadReuseKey,
        base_instructions: String,
        developer_instructions: String,
    ) {
        self.remove_thread_from_indexes(&thread_id);
        self.threads.insert(
            thread_id.clone(),
            RegisteredThread {
                instructions: StoredThreadInstructions {
                    base_instructions,
                    developer_instructions,
                },
                answered_short_ids: BTreeSet::new(),
                canon_show_expectation_ids: BTreeSet::new(),
            },
        );
        self.threads_by_prerender_reuse_key
            .insert(prerender_reuse_key, thread_id.clone());
        self.threads_by_rendered_reuse_key
            .insert(rendered_reuse_key, thread_id);
    }

    pub(in crate::check::interrogation::session::thread) fn stored_thread_instructions(
        &self,
        thread_id: &str,
    ) -> Option<StoredThreadInstructions> {
        self.threads
            .get(thread_id)
            .map(|thread| thread.instructions.clone())
    }

    pub(in crate::check::interrogation::session::thread) fn clear_threads(&mut self) {
        self.threads.clear();
        self.threads_by_prerender_reuse_key.clear();
        self.threads_by_rendered_reuse_key.clear();
    }

    pub(in crate::check::interrogation::session::thread) fn discard_thread(
        &mut self,
        thread_id: &str,
    ) {
        self.threads.remove(thread_id);
        self.remove_thread_from_indexes(thread_id);
    }

    pub(in crate::check::interrogation::session::thread) fn retire_threads_after_turn(
        &mut self,
        retired_threads: Vec<String>,
    ) {
        if retired_threads.is_empty() {
            return;
        }
        let retired_threads = retired_threads.into_iter().collect::<BTreeSet<_>>();
        self.threads
            .retain(|thread_id, _| !retired_threads.contains(thread_id));
        self.retain_valid_indexes();
    }

    fn remove_thread_from_indexes(&mut self, thread_id: &str) {
        self.threads_by_prerender_reuse_key
            .retain(|_, indexed_thread_id| indexed_thread_id != thread_id);
        self.threads_by_rendered_reuse_key
            .retain(|_, indexed_thread_id| indexed_thread_id != thread_id);
    }

    fn retain_valid_indexes(&mut self) {
        let threads = &self.threads;
        self.threads_by_prerender_reuse_key
            .retain(|_, thread_id| threads.contains_key(thread_id));
        self.threads_by_rendered_reuse_key
            .retain(|_, thread_id| threads.contains_key(thread_id));
    }
}
