use crate::check::core::ResolvedExpectation;
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::{InterrogationSession, InterrogationTurnKind};
use crate::check::EvaluatorResponseSchemaScope;
use crate::config_types::AgentConfig;
use crate::evaluator::{
    BaseInstructionsContext, DeveloperInstructionsCacheKey, EvaluatorAttempt,
    EvaluatorAttemptSequence, EvaluatorError, EvaluatorPromptMode, EvaluatorRunner, PromptRenderer,
    ThreadLifecycleLog,
};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;
use std::sync::Arc;

pub(crate) struct ThreadTurnContext<'ctx, 'runtime, 'log, R: EvaluatorRunner> {
    pub(crate) runtime: &'ctx CheckRuntime<'runtime>,
    pub(crate) runner: &'ctx mut R,
    pub(crate) diagnostic_log: &'ctx mut Option<&'log mut DiagnosticLogWriter>,
    pub(crate) visible_tree_oid_cache: &'ctx mut crate::git::VisibleTreeOidCache,
    pub(crate) interrogation_session: &'ctx mut InterrogationSession,
    pub(crate) xpec_state: &'ctx mut XpecStateCache,
    pub(crate) attempt_sequence: &'ctx mut EvaluatorAttemptSequence,
}

#[derive(Clone)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) attempt: EvaluatorAttempt,
    pub(crate) agent: &'a AgentConfig,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) visible_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) response_contract: ThreadTurnResponseContract,
    pub(crate) diff_target_prior_answer: Option<&'a str>,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) short_id: &'a str,
    pub(crate) question_context: &'a str,
    pub(crate) prompt_mode: EvaluatorPromptMode<'a>,
    pub(crate) task_input: &'a str,
    pub(crate) prompt_renderer: Arc<PromptRenderer>,
    pub(crate) progress: Option<&'a crate::evaluator::EvaluatorProgress>,
}

impl ThreadTurnRequest<'_> {
    pub(super) fn canon_show_dynamic_tools_enabled(&self) -> bool {
        self.expectation_id.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ThreadInstructionReuseKey {
    pub(super) base_context: BaseInstructionsContext,
    pub(super) developer_cache_key: DeveloperInstructionsCacheKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadTurnResponseContract {
    ExpectationResult,
    AdHocQuestion,
    FixedQScope,
    QScopeVerification,
}

impl ThreadTurnResponseContract {
    pub(super) fn for_expectation_turn(
        expectation: &ResolvedExpectation,
        turn_kind: InterrogationTurnKind,
    ) -> ThreadTurnResponseContract {
        if turn_kind == InterrogationTurnKind::QScopeVerification {
            return ThreadTurnResponseContract::QScopeVerification;
        }
        if !expectation.q_scope.is_auto() {
            ThreadTurnResponseContract::FixedQScope
        } else if expectation.is_temporary_query() {
            ThreadTurnResponseContract::AdHocQuestion
        } else {
            ThreadTurnResponseContract::ExpectationResult
        }
    }

    pub(in crate::check::interrogation::session::thread) fn schema_scope(
        self,
        runtime: &CheckRuntime<'_>,
        enforced_scope: &[String],
    ) -> EvaluatorResponseSchemaScope {
        if !self.q_scope_is_auto() {
            // A configured path-list q-scope is fixed for every turn. Because
            // policy cannot widen or narrow it, neither ScopeTooNarrow nor a
            // qScopeSuggestion belongs in the response contract.
            return EvaluatorResponseSchemaScope::FixedQScope;
        }
        if runtime.evaluator_interrogations_never_hide_files() {
            // With no hidden files there is likewise no scope to negotiate.
            return EvaluatorResponseSchemaScope::NoHiddenFiles;
        }
        if self == ThreadTurnResponseContract::QScopeVerification {
            return EvaluatorResponseSchemaScope::AutoRestricted;
        }
        EvaluatorResponseSchemaScope::for_auto_q_scope(enforced_scope)
    }

    pub(super) fn q_scope_is_auto(self) -> bool {
        self != ThreadTurnResponseContract::FixedQScope
    }

    pub(super) fn is_q_scope_verification(self) -> bool {
        self == ThreadTurnResponseContract::QScopeVerification
    }
}

pub(super) fn evaluator_prompt_mode<'a>(
    runtime: &'a CheckRuntime<'a>,
    diff_from_tree_oid: Option<&'a str>,
    target_is_diff: bool,
) -> Result<EvaluatorPromptMode<'a>, EvaluatorError> {
    match (
        diff_from_tree_oid,
        runtime.git_checked_tree_oid(),
        runtime.is_in_place(),
    ) {
        (Some(base_tree_oid), Some(checked_tree_oid), false) => Ok(EvaluatorPromptMode::GitDiff {
            target_is_diff,
            base_tree_oid,
            checked_tree_oid,
            git_environment: runtime.prompt_git_environment(),
        }),
        (None, None, true) => Ok(EvaluatorPromptMode::InPlace),
        _ => Err(EvaluatorError::message(
            "inconsistent evaluator view and Git diff context",
        )),
    }
}

pub(in crate::check::interrogation::session::thread) struct ThreadSelection {
    pub(in crate::check::interrogation::session::thread) lifecycle_log: ThreadLifecycleLog,
    pub(in crate::check::interrogation::session::thread) reused_existing_thread: bool,
}
