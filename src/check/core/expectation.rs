use crate::config_types::{
    AgentConfig, Cooldown, Expectation, ExpectationTarget, ExpectationTo, QScope,
};

#[derive(Debug, Clone)]
pub(crate) enum ResolvedExpectationKind {
    Configured { id: String },
    TemporaryQuery,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedExpectation {
    pub(crate) kind: ResolvedExpectationKind,
    pub(crate) display_id: String,
    pub(crate) to: ExpectationTo,
    // [H9] Final canon check order uses this ascending before latest fail time.
    pub(crate) rank: i64,
    pub(crate) question: String,
    pub(crate) expected_answer: String,
    pub(crate) question_context: String,
    // Literal `diff-from` config selection. The interrogation session resolves
    // it to the prompt diff tree with the active runtime and last-pass state.
    pub(crate) diff_from: String,
    pub(crate) target: Option<ExpectationTarget>,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
    pub(crate) q_scope: QScope,
}

impl ResolvedExpectation {
    pub(crate) fn configured_id(&self) -> Option<&str> {
        match &self.kind {
            ResolvedExpectationKind::Configured { id } => Some(id),
            ResolvedExpectationKind::TemporaryQuery => None,
        }
    }

    pub(crate) fn require_configured_id(&self) -> Result<&str, String> {
        self.configured_id()
            .ok_or_else(|| "temporary query has no configured expectation ID".to_string())
    }

    pub(crate) fn is_temporary_query(&self) -> bool {
        matches!(self.kind, ResolvedExpectationKind::TemporaryQuery)
    }

    pub(crate) fn expected_answer(&self) -> &str {
        &self.expected_answer
    }

    pub(crate) fn from_configured(
        id: String,
        display_id: String,
        expectation: &Expectation,
    ) -> ResolvedExpectation {
        Self::from_configured_fields(
            ResolvedExpectationKind::Configured { id },
            display_id,
            expectation,
        )
    }

    pub(crate) fn from_resolved_ask_xpec(expectation: &Expectation) -> ResolvedExpectation {
        Self::from_configured_fields(
            ResolvedExpectationKind::TemporaryQuery,
            "q".to_string(),
            expectation,
        )
    }

    fn from_configured_fields(
        kind: ResolvedExpectationKind,
        display_id: String,
        expectation: &Expectation,
    ) -> ResolvedExpectation {
        ResolvedExpectation {
            kind,
            display_id,
            to: expectation.to,
            rank: expectation.rank,
            question: expectation.q.clone(),
            expected_answer: expectation.a.clone(),
            question_context: expectation.question_context.clone(),
            diff_from: expectation.diff_from.clone(),
            target: expectation.target.clone(),
            agent: expectation.agent.clone(),
            cooldown: expectation.cooldown,
            q_scope: expectation.q_scope.clone(),
        }
    }
}
