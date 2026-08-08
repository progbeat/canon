use super::*;
use crate::check::core::{ResolvedExpectationKind, ERROR_INVALID_QUESTION};
use crate::config_types::{AgentConfig, ExpectationTo};
use std::collections::VecDeque;

#[test] // xpec: kg,UR,EL,fc
fn restricted_scope_error_retries_once_at_full_project_scope() {
    let restricted_scope = vec!["src/old".to_string()];
    let mut runner = FakePolicyTurnRunner::new([
        FakeInterrogation::scope_error(restricted_scope.clone()),
        FakeInterrogation::passing(full_scope(), None),
    ]);
    let mut current_scope = restricted_scope.clone();

    let completed = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut current_scope,
        None,
    )
    .unwrap();

    assert_eq!(completed.interrogation.scope, full_scope());
    assert_eq!(current_scope, full_scope());
    assert_eq!(
        runner.calls,
        [
            (InterrogationTurnKind::Initial, restricted_scope),
            (InterrogationTurnKind::FullScopeRetry, full_scope()),
        ]
    );
    assert!(runner.narrowing_events.is_empty());
}

#[test] // xpec: kg
#[should_panic(expected = "ScopeTooNarrow error on full project scope")]
fn full_project_scope_error_triggers_the_canonical_assertion() {
    let mut runner = FakePolicyTurnRunner::new([FakeInterrogation::scope_error(full_scope())]);
    let mut current_scope = full_scope();

    let _ = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut current_scope,
        None,
    );
}

#[test] // xpec: kg,UR,5
fn verification_answer_replaces_the_initial_pass() {
    let proposed_scope = vec!["src/focused".to_string()];
    let mut runner = FakePolicyTurnRunner::new([
        FakeInterrogation::passing(full_scope(), Some(proposed_scope.clone())),
        FakeInterrogation::wrong_answer(proposed_scope.clone(), None),
    ]);
    let mut current_scope = full_scope();

    let completed = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut current_scope,
        None,
    )
    .unwrap();

    assert_eq!(completed.interrogation.scope, proposed_scope);
    assert!(!completed.interrogation.answer_matches_expected);
    assert_eq!(current_scope, completed.interrogation.scope);
    assert_eq!(
        runner.narrowing_events,
        [(full_scope(), current_scope, true)]
    );
}

#[test] // xpec: kg,UR,5
fn verification_error_keeps_the_initial_pass_and_scope() {
    let proposed_scope = vec!["src/focused".to_string()];
    let mut runner = FakePolicyTurnRunner::new([
        FakeInterrogation::passing(full_scope(), Some(proposed_scope.clone())),
        FakeInterrogation::error(proposed_scope.clone(), ERROR_INVALID_QUESTION),
    ]);
    let mut current_scope = full_scope();

    let completed = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut current_scope,
        None,
    )
    .unwrap();

    assert_eq!(completed.interrogation.scope, full_scope());
    assert!(completed.interrogation.error.is_none());
    assert_eq!(current_scope, full_scope());
    assert_eq!(
        runner.narrowing_events,
        [(full_scope(), proposed_scope, false)]
    );
}

#[test] // xpec: kg,UR,5
fn passing_answer_without_a_usable_suggestion_remains_final() {
    let mut runner = FakePolicyTurnRunner::discarding_suggestions([FakeInterrogation::passing(
        full_scope(),
        Some(vec!["missing".to_string()]),
    )]);
    let mut current_scope = full_scope();

    let completed = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut current_scope,
        None,
    )
    .unwrap();

    assert_eq!(completed.interrogation.scope, full_scope());
    assert!(completed.interrogation.error.is_none());
    assert_eq!(
        runner.calls,
        [(InterrogationTurnKind::Initial, full_scope())]
    );
}

#[test] // xpec: kg,UR,5
fn wrong_answer_does_not_trigger_q_scope_verification() {
    let current_scope = vec!["src/current".to_string()];
    let mut runner = FakePolicyTurnRunner::new([FakeInterrogation::wrong_answer(
        current_scope.clone(),
        Some(vec!["src/proposed".to_string()]),
    )]);
    let mut resulting_scope = current_scope.clone();

    let completed = run_started_policy_interrogation(
        &mut runner,
        &auto_scope_expectation(),
        &mut resulting_scope,
        None,
    )
    .unwrap();

    assert_eq!(completed.interrogation.scope, current_scope);
    assert_eq!(resulting_scope, completed.interrogation.scope);
    assert_eq!(
        runner.calls,
        [(InterrogationTurnKind::Initial, current_scope)]
    );
}

struct FakePolicyTurnRunner {
    responses: VecDeque<FakeInterrogation>,
    calls: Vec<(InterrogationTurnKind, Vec<String>)>,
    narrowing_events: Vec<(Vec<String>, Vec<String>, bool)>,
    accept_suggestions: bool,
}

impl FakePolicyTurnRunner {
    fn new(responses: impl IntoIterator<Item = FakeInterrogation>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: Vec::new(),
            narrowing_events: Vec::new(),
            accept_suggestions: true,
        }
    }

    fn discarding_suggestions(responses: impl IntoIterator<Item = FakeInterrogation>) -> Self {
        Self {
            accept_suggestions: false,
            ..Self::new(responses)
        }
    }
}

impl PolicyTurnRunner for FakePolicyTurnRunner {
    type Interrogation = FakeInterrogation;

    fn evaluator_turns_may_hide_files(&self) -> bool {
        true
    }

    fn run_policy_turn(
        &mut self,
        _expectation: &ResolvedExpectation,
        scope: &[String],
        turn_kind: InterrogationTurnKind,
        _progress: Option<&EvaluatorProgress>,
    ) -> Result<Self::Interrogation, String> {
        self.calls.push((turn_kind, scope.to_vec()));
        self.responses
            .pop_front()
            .ok_or_else(|| "missing fake policy response".to_string())
    }

    fn write_scope_narrowing(
        &mut self,
        _expectation_id: Option<&str>,
        current_scope: &[String],
        proposed_scope: &[String],
        accepted: bool,
    ) -> Result<(), String> {
        self.narrowing_events
            .push((current_scope.to_vec(), proposed_scope.to_vec(), accepted));
        Ok(())
    }

    fn q_scope_verification_scope(
        &mut self,
        _expectation: &ResolvedExpectation,
        suggestion: Option<&[String]>,
        _current_scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        Ok(if self.accept_suggestions {
            suggestion.map(<[String]>::to_vec)
        } else {
            None
        })
    }
}

struct FakeInterrogation {
    error: Option<String>,
    answer_matches_expected: bool,
    scope: Vec<String>,
    suggestion: Option<Vec<String>>,
}

impl FakeInterrogation {
    fn scope_error(scope: Vec<String>) -> Self {
        Self::error(scope, ERROR_SCOPE_TOO_NARROW)
    }

    fn error(scope: Vec<String>, error: &str) -> Self {
        Self {
            error: Some(error.to_string()),
            answer_matches_expected: false,
            suggestion: Some(vec!["src/repaired".to_string()]),
            scope,
        }
    }

    fn passing(scope: Vec<String>, suggestion: Option<Vec<String>>) -> Self {
        Self {
            error: None,
            answer_matches_expected: true,
            scope,
            suggestion,
        }
    }

    fn wrong_answer(scope: Vec<String>, suggestion: Option<Vec<String>>) -> Self {
        Self {
            error: None,
            answer_matches_expected: false,
            scope,
            suggestion,
        }
    }
}

impl PolicyInterrogation for FakeInterrogation {
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl PolicyTurnMetadataSource for FakeInterrogation {
    fn turn_metadata(&self) -> model::PolicyTurnMetadata {
        model::PolicyTurnMetadata {
            context_compacted: false,
            interrupted: false,
        }
    }
}

impl StartedPolicyInterrogation for FakeInterrogation {
    fn merge_initial_turn_metadata(&mut self, _initial: &Self) {}

    fn recorded_q_scope(&self) -> &[String] {
        &self.scope
    }

    fn q_scope_suggestion(&self) -> Option<&[String]> {
        self.suggestion.as_deref()
    }

    fn has_passing_answer_for_q_scope_verification(&self) -> bool {
        self.error.is_none() && self.answer_matches_expected
    }
}

fn auto_scope_expectation() -> ResolvedExpectation {
    ResolvedExpectation {
        kind: ResolvedExpectationKind::Configured {
            id: "expectation".to_string(),
        },
        display_id: "e".to_string(),
        to: ExpectationTo::Agent,
        rank: 0,
        question: "Does recovery finish at the canonical scope?".to_string(),
        expected_answer: "yes".to_string(),
        question_context: String::new(),
        diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
        target: None,
        agent: AgentConfig::default(),
        cooldown: None,
        q_scope: Default::default(),
    }
}
