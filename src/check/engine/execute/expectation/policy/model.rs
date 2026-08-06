use crate::check::core::{
    CheckRecord, InterrogationAnswer, InterrogationResult, InterrogationTurn,
};
use crate::check::interrogation::policy::interrogation_has_passing_answer_for_q_scope_verification;

pub(in crate::check::engine::execute::expectation) trait PolicyInterrogation {
    fn error(&self) -> Option<&str>;

    fn answer_returned(&self) -> bool {
        self.error().is_none()
    }
}

pub(super) struct PolicyTurnMetadata {
    pub(super) context_compacted: bool,
    pub(super) interrupted: bool,
}

impl PolicyTurnMetadata {
    pub(super) fn include(&mut self, turn: PolicyTurnMetadata) {
        self.context_compacted |= turn.context_compacted;
        self.interrupted |= turn.interrupted;
    }
}

pub(super) trait PolicyTurnMetadataSource {
    fn turn_metadata(&self) -> PolicyTurnMetadata;
}

impl<T> PolicyTurnMetadataSource for InterrogationTurn<T> {
    fn turn_metadata(&self) -> PolicyTurnMetadata {
        PolicyTurnMetadata {
            context_compacted: self.context_compacted,
            interrupted: self.interrupted,
        }
    }
}

pub(super) trait StartedPolicyInterrogation:
    PolicyInterrogation + PolicyTurnMetadataSource
{
    fn merge_initial_turn_metadata(&mut self, initial: &Self);
    fn recorded_q_scope(&self) -> &[String];
    fn q_scope_suggestion(&self) -> Option<&[String]>;
    fn has_passing_answer_for_q_scope_verification(&self) -> bool;
}

impl PolicyInterrogation for InterrogationAnswer {
    fn error(&self) -> Option<&str> {
        self.output.answer.error.as_deref()
    }
}

impl StartedPolicyInterrogation for InterrogationAnswer {
    fn merge_initial_turn_metadata(&mut self, initial: &Self) {
        merge_interrogation_turn_metadata(self, initial);
    }

    fn recorded_q_scope(&self) -> &[String] {
        &self.output.answer.scope
    }

    fn q_scope_suggestion(&self) -> Option<&[String]> {
        self.output.answer.q_scope_suggestion.as_deref()
    }

    fn has_passing_answer_for_q_scope_verification(&self) -> bool {
        // [l] A temporary query's expected answer is empty, so no valid agent
        // answer passes check_answer. Its q-scope suggestion is returned to the
        // user without an independent verification turn.
        false
    }
}

impl PolicyInterrogation for CheckRecord {
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl PolicyInterrogation for InterrogationResult {
    fn error(&self) -> Option<&str> {
        self.output.error()
    }
}

impl StartedPolicyInterrogation for InterrogationResult {
    fn merge_initial_turn_metadata(&mut self, initial: &Self) {
        merge_interrogation_turn_metadata(self, initial);
    }

    fn recorded_q_scope(&self) -> &[String] {
        &self.output.scope
    }

    fn q_scope_suggestion(&self) -> Option<&[String]> {
        self.output.q_scope_suggestion.as_deref()
    }

    fn has_passing_answer_for_q_scope_verification(&self) -> bool {
        interrogation_has_passing_answer_for_q_scope_verification(self)
    }
}

fn merge_interrogation_turn_metadata<T>(
    retry: &mut InterrogationTurn<T>,
    initial: &InterrogationTurn<T>,
) {
    retry.context_compacted |= initial.context_compacted;
    retry.interrupted |= initial.interrupted;
}
