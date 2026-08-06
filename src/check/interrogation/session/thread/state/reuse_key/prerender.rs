use super::super::super::model::ThreadInstructionReuseKey;
use super::super::workspace::ThreadWorkspace;
use crate::evaluator::EvaluatorThreadConfigIdentity;

pub(in crate::check::interrogation::session::thread) struct PrerenderEvaluatorThreadReuseKeyContext<
    'a,
> {
    pub(in crate::check::interrogation::session::thread) evaluator_config_identity:
        &'a EvaluatorThreadConfigIdentity,
    pub(in crate::check::interrogation::session::thread) workspace: &'a ThreadWorkspace,
    pub(in crate::check::interrogation::session::thread) instruction_reuse_key:
        &'a ThreadInstructionReuseKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::check::interrogation::session::thread) struct PrerenderEvaluatorThreadReuseKey {
    evaluator_config_identity: EvaluatorThreadConfigIdentity,
    workspace: ThreadWorkspace,
    instruction_reuse_key: ThreadInstructionReuseKey,
}

pub(in crate::check::interrogation::session::thread) fn prerender_evaluator_thread_reuse_key(
    context: PrerenderEvaluatorThreadReuseKeyContext<'_>,
) -> PrerenderEvaluatorThreadReuseKey {
    // This is the cheap pre-render lookup key. The evaluator supplies its
    // opaque effective-config identity; this thread component owns workspace
    // identity and the stable instruction-rendering inputs.
    // The turn prompt is deliberately excluded: each evaluator turn supplies its
    // own task input, while a reusable thread keeps only the base/developer
    // instruction context from thread startup.
    PrerenderEvaluatorThreadReuseKey {
        evaluator_config_identity: context.evaluator_config_identity.clone(),
        workspace: context.workspace.clone(),
        instruction_reuse_key: context.instruction_reuse_key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::make_evaluator_thread_config_identity;
    use super::*;
    use crate::config_types::AgentConfig;
    use crate::evaluator::{
        developer_instructions_cache_key, BaseInstructionsContext, DeveloperInstructionsContext,
        EvaluatorPromptMode,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[derive(Clone)]
    struct PrerenderEvaluatorThreadReuseKeyInputs {
        root: PathBuf,
        base_tree_oid: String,
        checked_tree_oid: String,
        git_environment: Vec<(OsString, OsString)>,
        question_context: String,
        visible_scope: Vec<String>,
        num_invisible_files: usize,
        q_scope_is_full_project: bool,
        q_scope_is_auto: bool,
        target_is_diff: bool,
        in_place: bool,
        evaluator_config_identity: EvaluatorThreadConfigIdentity,
        workspace: ThreadWorkspace,
    }

    impl PrerenderEvaluatorThreadReuseKeyInputs {
        fn with_change(&self, change: impl FnOnce(&mut Self)) -> Self {
            let mut changed = self.clone();
            change(&mut changed);
            changed
        }
    }

    fn make_thread_instruction_reuse_key(
        inputs: &PrerenderEvaluatorThreadReuseKeyInputs,
    ) -> ThreadInstructionReuseKey {
        let prompt_mode = if inputs.in_place {
            EvaluatorPromptMode::InPlace
        } else {
            EvaluatorPromptMode::GitDiff {
                target_is_diff: inputs.target_is_diff,
                base_tree_oid: &inputs.base_tree_oid,
                checked_tree_oid: &inputs.checked_tree_oid,
                git_environment: &inputs.git_environment,
            }
        };
        ThreadInstructionReuseKey {
            base_context: BaseInstructionsContext {
                in_place: inputs.in_place,
                q_scope_is_full_project: inputs.q_scope_is_full_project,
                q_scope_is_auto: inputs.q_scope_is_auto,
                q_scope_verification: false,
            },
            developer_cache_key: developer_instructions_cache_key(&DeveloperInstructionsContext {
                root: &inputs.root,
                mode: prompt_mode,
                question_context: &inputs.question_context,
                visible_scope: &inputs.visible_scope,
                num_invisible_files: inputs.num_invisible_files,
            }),
        }
    }

    fn make_prerender_evaluator_thread_reuse_key(
        inputs: &PrerenderEvaluatorThreadReuseKeyInputs,
    ) -> PrerenderEvaluatorThreadReuseKey {
        let instruction_reuse_key = make_thread_instruction_reuse_key(inputs);
        prerender_evaluator_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
            evaluator_config_identity: &inputs.evaluator_config_identity,
            workspace: &inputs.workspace,
            instruction_reuse_key: &instruction_reuse_key,
        })
    }

    fn baseline_prerender_evaluator_thread_reuse_key_inputs(
        agent: &AgentConfig,
    ) -> PrerenderEvaluatorThreadReuseKeyInputs {
        PrerenderEvaluatorThreadReuseKeyInputs {
            root: PathBuf::from("root-a"),
            base_tree_oid: "base-a".to_string(),
            checked_tree_oid: "checked-a".to_string(),
            git_environment: vec![(OsString::from("GIT_INDEX_FILE"), OsString::from("index-a"))],
            question_context: "instructions-a".to_string(),
            visible_scope: vec!["scope-a".to_string()],
            num_invisible_files: 1,
            q_scope_is_full_project: false,
            q_scope_is_auto: true,
            target_is_diff: false,
            in_place: false,
            evaluator_config_identity: make_evaluator_thread_config_identity(
                agent,
                None,
                "medium",
                &[],
            ),
            workspace: ThreadWorkspace::git_for_test("visible-a"),
        }
    }

    #[test] // xpec: d,fD
    fn prerender_evaluator_thread_reuse_key_includes_all_inputs() {
        let agent = AgentConfig::default();
        let baseline_inputs = baseline_prerender_evaluator_thread_reuse_key_inputs(&agent);
        let baseline_prerender_evaluator_thread_reuse_key =
            make_prerender_evaluator_thread_reuse_key(&baseline_inputs);
        let different_reuse_key_inputs = vec![
            baseline_inputs.with_change(|inputs| inputs.root = PathBuf::from("root-b")),
            baseline_inputs.with_change(|inputs| inputs.base_tree_oid = "base-b".to_string()),
            baseline_inputs.with_change(|inputs| inputs.checked_tree_oid = "checked-b".to_string()),
            baseline_inputs.with_change(|inputs| {
                inputs.git_environment =
                    vec![(OsString::from("GIT_INDEX_FILE"), OsString::from("index-b"))];
            }),
            baseline_inputs
                .with_change(|inputs| inputs.question_context = "instructions-b".to_string()),
            baseline_inputs
                .with_change(|inputs| inputs.visible_scope = vec!["scope-b".to_string()]),
            baseline_inputs.with_change(|inputs| inputs.num_invisible_files = 2),
            baseline_inputs.with_change(|inputs| inputs.q_scope_is_full_project = true),
            baseline_inputs.with_change(|inputs| inputs.q_scope_is_auto = false),
            baseline_inputs.with_change(|inputs| inputs.target_is_diff = true),
            baseline_inputs.with_change(|inputs| inputs.in_place = true),
            baseline_inputs.with_change(|inputs| {
                inputs.evaluator_config_identity =
                    make_evaluator_thread_config_identity(&agent, None, "high", &[]);
            }),
            baseline_inputs.with_change(|inputs| {
                inputs.workspace = ThreadWorkspace::git_for_test("visible-b");
            }),
        ];

        for inputs in different_reuse_key_inputs {
            assert_ne!(
                baseline_prerender_evaluator_thread_reuse_key,
                make_prerender_evaluator_thread_reuse_key(&inputs)
            );
        }
    }
}
