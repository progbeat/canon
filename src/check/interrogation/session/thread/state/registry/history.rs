use super::ThreadRegistry;
use std::collections::BTreeSet;

impl ThreadRegistry {
    pub(in crate::check::interrogation::session::thread) fn thread_answered_short_ids(
        &self,
        thread_id: &str,
    ) -> Vec<String> {
        self.threads
            .get(thread_id)
            .map(|thread| thread.answered_short_ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(in crate::check::interrogation::session::thread) fn record_thread_answered_short_id(
        &mut self,
        thread_id: &str,
        short_id: &str,
    ) {
        if let Some(thread) = self.threads.get_mut(thread_id) {
            thread.answered_short_ids.insert(short_id.to_string());
        }
    }

    pub(super) fn thread_has_seen_canon_show_expectation(
        &self,
        thread_id: &str,
        expectation_id: Option<&str>,
    ) -> bool {
        let Some(expectation_id) = expectation_id else {
            return false;
        };
        self.threads
            .get(thread_id)
            .is_some_and(|thread| thread.canon_show_expectation_ids.contains(expectation_id))
    }

    pub(in crate::check::interrogation::session::thread) fn record_thread_canon_show_expectation_ids(
        &mut self,
        thread_id: &str,
        expectation_ids: BTreeSet<String>,
    ) {
        if expectation_ids.is_empty() {
            return;
        }
        if let Some(thread) = self.threads.get_mut(thread_id) {
            thread.canon_show_expectation_ids.extend(expectation_ids);
        }
    }
}
