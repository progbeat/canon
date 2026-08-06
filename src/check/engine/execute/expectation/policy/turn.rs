use super::PolicyInterrogation;
use crate::check::core::ResolvedExpectation;
use crate::check::interrogation::policy::{
    interrogate_or_error, q_scope_suggestion_scope_for_independent_verification,
    write_scope_narrowing_event, InterrogationCall, RecoverableInterrogation,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::{InterrogationSession, InterrogationTurnKind};
use crate::check::CheckRunCaches;
use crate::evaluator::{EvaluatorProgress, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use std::marker::PhantomData;

pub(super) struct PolicyTurnContext<'a, 'log, R: EvaluatorRunner, O> {
    runtime: &'a CheckRuntime<'a>,
    runner: &'a mut R,
    diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    caches: &'a mut CheckRunCaches,
    interrogation_session: &'a mut InterrogationSession,
    output: PhantomData<fn() -> O>,
}

impl<'a, 'log, R: EvaluatorRunner, O> PolicyTurnContext<'a, 'log, R, O> {
    pub(super) fn new(
        runtime: &'a CheckRuntime<'a>,
        runner: &'a mut R,
        diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
        caches: &'a mut CheckRunCaches,
        interrogation_session: &'a mut InterrogationSession,
    ) -> Self {
        Self {
            runtime,
            runner,
            diagnostic_log,
            caches,
            interrogation_session,
            output: PhantomData,
        }
    }
}

pub(super) trait PolicyTurnRunner {
    type Interrogation: PolicyInterrogation;

    fn evaluator_turns_may_hide_files(&self) -> bool;

    fn run_policy_turn(
        &mut self,
        expectation: &ResolvedExpectation,
        scope: &[String],
        turn_kind: InterrogationTurnKind,
        progress: Option<&EvaluatorProgress>,
    ) -> Result<Self::Interrogation, String>;

    fn write_scope_narrowing(
        &mut self,
        expectation_id: Option<&str>,
        current_scope: &[String],
        proposed_scope: &[String],
        accepted: bool,
    ) -> Result<(), String>;

    fn q_scope_verification_scope(
        &mut self,
        expectation: &ResolvedExpectation,
        suggestion: Option<&[String]>,
        current_scope: &[String],
    ) -> Result<Option<Vec<String>>, String>;
}

impl<R, O> PolicyTurnRunner for PolicyTurnContext<'_, '_, R, O>
where
    R: EvaluatorRunner,
    O: PolicyInterrogation + RecoverableInterrogation,
{
    type Interrogation = O;

    fn evaluator_turns_may_hide_files(&self) -> bool {
        !self.runtime.evaluator_interrogations_never_hide_files()
    }

    fn run_policy_turn(
        &mut self,
        expectation: &ResolvedExpectation,
        scope: &[String],
        turn_kind: InterrogationTurnKind,
        progress: Option<&EvaluatorProgress>,
    ) -> Result<Self::Interrogation, String> {
        interrogate_or_error(
            InterrogationCall {
                runtime: self.runtime,
                expectation,
                scope,
                turn_kind,
                progress,
            },
            self.runner,
            self.diagnostic_log,
            self.interrogation_session,
            &mut self.caches.xpec_state,
            &mut self.caches.visible_tree_oid_cache,
        )
    }

    fn write_scope_narrowing(
        &mut self,
        expectation_id: Option<&str>,
        current_scope: &[String],
        proposed_scope: &[String],
        accepted: bool,
    ) -> Result<(), String> {
        write_scope_narrowing_event(
            self.diagnostic_log,
            expectation_id,
            current_scope,
            proposed_scope,
            accepted,
        )
    }

    fn q_scope_verification_scope(
        &mut self,
        expectation: &ResolvedExpectation,
        suggestion: Option<&[String]>,
        current_scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        q_scope_suggestion_scope_for_independent_verification(
            self.runtime,
            &expectation.agent,
            suggestion,
            current_scope,
            &mut self.caches.visible_tree_oid_cache,
        )
    }
}
