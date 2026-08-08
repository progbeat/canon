use super::super::workspace::ThreadWorkspace;
use crate::evaluator::EvaluatorThreadConfigIdentity;

pub(in crate::check::interrogation::session::thread) struct RenderedEvaluatorThreadReuseKeyContext<
    'a,
> {
    pub(in crate::check::interrogation::session::thread) evaluator_config_identity:
        &'a EvaluatorThreadConfigIdentity,
    pub(in crate::check::interrogation::session::thread) workspace: &'a ThreadWorkspace,
    pub(in crate::check::interrogation::session::thread) base_instructions: &'a str,
    pub(in crate::check::interrogation::session::thread) developer_instructions: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::check::interrogation::session::thread) struct RenderedEvaluatorThreadReuseKey {
    evaluator_config_identity: EvaluatorThreadConfigIdentity,
    workspace: ThreadWorkspace,
    base_instructions: String,
    developer_instructions: String,
}

pub(in crate::check::interrogation::session::thread) fn rendered_evaluator_thread_reuse_key(
    context: RenderedEvaluatorThreadReuseKeyContext<'_>,
) -> RenderedEvaluatorThreadReuseKey {
    RenderedEvaluatorThreadReuseKey {
        evaluator_config_identity: context.evaluator_config_identity.clone(),
        workspace: context.workspace.clone(),
        base_instructions: context.base_instructions.to_string(),
        developer_instructions: context.developer_instructions.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::make_evaluator_thread_config_identity;
    use super::*;
    use crate::config_types::AgentConfig;

    #[test] // xpec: gN,fD
    fn rendered_evaluator_thread_reuse_key_includes_all_inputs() {
        let agent = AgentConfig::default();
        let evaluator_config_identity_a =
            make_evaluator_thread_config_identity(&agent, None, "medium", &[]);
        let evaluator_config_identity_b =
            make_evaluator_thread_config_identity(&agent, None, "high", &[]);
        let workspace_a = ThreadWorkspace::git_for_test("visible-a");
        let workspace_b = ThreadWorkspace::git_for_test("visible-b");
        let make_rendered_evaluator_thread_reuse_key =
            |evaluator_config_identity, workspace, base_instructions, developer_instructions| {
                rendered_evaluator_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
                    evaluator_config_identity,
                    workspace,
                    base_instructions,
                    developer_instructions,
                })
            };
        let baseline_rendered_evaluator_thread_reuse_key = make_rendered_evaluator_thread_reuse_key(
            &evaluator_config_identity_a,
            &workspace_a,
            "base-a",
            "developer-a",
        );

        assert_ne!(
            baseline_rendered_evaluator_thread_reuse_key,
            make_rendered_evaluator_thread_reuse_key(
                &evaluator_config_identity_a,
                &workspace_a,
                "base-b",
                "developer-a"
            )
        );
        assert_ne!(
            baseline_rendered_evaluator_thread_reuse_key,
            make_rendered_evaluator_thread_reuse_key(
                &evaluator_config_identity_a,
                &workspace_a,
                "base-a",
                "developer-b"
            )
        );
        assert_ne!(
            baseline_rendered_evaluator_thread_reuse_key,
            make_rendered_evaluator_thread_reuse_key(
                &evaluator_config_identity_b,
                &workspace_a,
                "base-a",
                "developer-a"
            )
        );
        assert_ne!(
            baseline_rendered_evaluator_thread_reuse_key,
            make_rendered_evaluator_thread_reuse_key(
                &evaluator_config_identity_a,
                &workspace_b,
                "base-a",
                "developer-a"
            )
        );
    }
}
