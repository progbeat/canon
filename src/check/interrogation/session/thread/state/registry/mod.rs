mod history;
mod reuse;

use super::reuse_key::{PrerenderEvaluatorThreadReuseKey, RenderedEvaluatorThreadReuseKey};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub(in crate::check::interrogation::session::thread) struct ThreadRegistry {
    // This is a run-level pool of evaluator threads, not one thread. The
    // pre-render key is used before rendering instructions only to avoid
    // rendering solely for the lookup.
    threads_by_prerender_reuse_key: HashMap<PrerenderEvaluatorThreadReuseKey, String>,
    // After instructions have been rendered anyway, this key lets identical
    // rendered instructions and startup context reuse a live evaluator thread.
    threads_by_rendered_reuse_key: HashMap<RenderedEvaluatorThreadReuseKey, String>,
    threads: BTreeMap<String, RegisteredThread>,
}

struct RegisteredThread {
    instructions: StoredThreadInstructions,
    answered_short_ids: BTreeSet<String>,
    // xpec: F
    // Per-thread memory of expectation IDs whose `canon.show` output reached
    // that evaluator thread; thread reuse filters consult this before reusing
    // the thread for any of those expectations.
    canon_show_expectation_ids: BTreeSet<String>,
}

#[derive(Clone)]
pub(in crate::check::interrogation::session::thread) struct StoredThreadInstructions {
    pub(in crate::check::interrogation::session::thread) base_instructions: String,
    pub(in crate::check::interrogation::session::thread) developer_instructions: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::interrogation::session::thread::model::ThreadInstructionReuseKey;
    use crate::check::interrogation::session::thread::state::{
        prerender_evaluator_thread_reuse_key, rendered_evaluator_thread_reuse_key,
        PrerenderEvaluatorThreadReuseKeyContext, RenderedEvaluatorThreadReuseKeyContext,
        ThreadWorkspace,
    };
    use crate::config_types::AgentConfig;
    use crate::evaluator::{
        developer_instructions_cache_key, evaluator_thread_config_identity,
        BaseInstructionsContext, DeveloperInstructionsContext, EvaluatorPromptMode,
        EvaluatorThreadConfigIdentityContext,
    };
    use std::path::Path;

    fn make_evaluator_thread_reuse_keys(
        label: &str,
    ) -> (
        PrerenderEvaluatorThreadReuseKey,
        RenderedEvaluatorThreadReuseKey,
    ) {
        let agent = AgentConfig::default();
        let scope = vec![".".to_string()];
        let workspace = ThreadWorkspace::in_place_for_test();
        let evaluator_config_identity =
            evaluator_thread_config_identity(EvaluatorThreadConfigIdentityContext {
                agent: &agent,
                model: None,
                thinking: "medium",
                dynamic_tools: &[],
            });
        let instruction_reuse_key = ThreadInstructionReuseKey {
            base_context: BaseInstructionsContext {
                in_place: true,
                q_scope_is_full_project: true,
                q_scope_is_auto: true,
                q_scope_verification: false,
            },
            developer_cache_key: developer_instructions_cache_key(&DeveloperInstructionsContext {
                root: Path::new("."),
                mode: EvaluatorPromptMode::InPlace,
                question_context: label,
                visible_scope: &scope,
                num_invisible_files: 0,
            }),
        };
        (
            prerender_evaluator_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
                evaluator_config_identity: &evaluator_config_identity,
                workspace: &workspace,
                instruction_reuse_key: &instruction_reuse_key,
            }),
            rendered_evaluator_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
                evaluator_config_identity: &evaluator_config_identity,
                workspace: &workspace,
                base_instructions: "base",
                developer_instructions: label,
            }),
        )
    }

    #[test] // xpec: F
    fn thread_canon_show_expectation_ids_are_thread_scoped() {
        let mut registry = ThreadRegistry::default();
        let (prerender_reuse_key, rendered_reuse_key) =
            make_evaluator_thread_reuse_keys("developer-a");
        registry.register_thread(
            "thread-a".to_string(),
            prerender_reuse_key,
            rendered_reuse_key,
            "base".to_string(),
            "developer".to_string(),
        );
        registry.record_thread_canon_show_expectation_ids(
            "thread-a",
            BTreeSet::from(["expectation-a".to_string()]),
        );

        assert!(registry.thread_has_seen_canon_show_expectation("thread-a", Some("expectation-a")));
        assert!(!registry.thread_has_seen_canon_show_expectation("thread-a", Some("expectation-b")));
        assert!(!registry.thread_has_seen_canon_show_expectation("thread-b", Some("expectation-a")));
        assert!(!registry.thread_has_seen_canon_show_expectation("thread-a", None));
    }

    #[test] // xpec: fD
    fn retiring_thread_removes_entry_and_both_reuse_indexes() {
        let mut registry = ThreadRegistry::default();
        let (prerender_reuse_key, rendered_reuse_key) =
            make_evaluator_thread_reuse_keys("developer-a");
        registry.register_thread(
            "thread-a".to_string(),
            prerender_reuse_key.clone(),
            rendered_reuse_key.clone(),
            "base".to_string(),
            "developer".to_string(),
        );

        assert_eq!(
            registry.reusable_thread_by_prerender_reuse_key(&prerender_reuse_key, None),
            Some("thread-a".to_string())
        );
        assert_eq!(
            registry.reusable_thread_by_rendered_reuse_key(&rendered_reuse_key, None),
            Some("thread-a".to_string())
        );
        registry.retire_threads_after_turn(vec!["thread-a".to_string()]);
        assert_eq!(
            registry.reusable_thread_by_prerender_reuse_key(&prerender_reuse_key, None),
            None
        );
        assert_eq!(
            registry.reusable_thread_by_rendered_reuse_key(&rendered_reuse_key, None),
            None
        );
        assert!(registry.stored_thread_instructions("thread-a").is_none());
    }

    #[test] // xpec: qv,fD
    fn discarding_one_thread_preserves_unrelated_thread_and_reuse_indexes() {
        let mut registry = ThreadRegistry::default();
        let (prerender_a, rendered_a) = make_evaluator_thread_reuse_keys("developer-a");
        let (prerender_b, rendered_b) = make_evaluator_thread_reuse_keys("developer-b");
        registry.register_thread(
            "thread-a".to_string(),
            prerender_a.clone(),
            rendered_a.clone(),
            "base".to_string(),
            "developer-a".to_string(),
        );
        registry.register_thread(
            "thread-b".to_string(),
            prerender_b.clone(),
            rendered_b.clone(),
            "base".to_string(),
            "developer-b".to_string(),
        );

        registry.discard_thread("thread-a");

        assert!(registry.stored_thread_instructions("thread-a").is_none());
        assert_eq!(
            registry.reusable_thread_by_prerender_reuse_key(&prerender_a, None),
            None
        );
        assert_eq!(
            registry.reusable_thread_by_rendered_reuse_key(&rendered_a, None),
            None
        );
        assert_eq!(
            registry.reusable_thread_by_prerender_reuse_key(&prerender_b, None),
            Some("thread-b".to_string())
        );
        assert_eq!(
            registry.reusable_thread_by_rendered_reuse_key(&rendered_b, None),
            Some("thread-b".to_string())
        );
    }
}
