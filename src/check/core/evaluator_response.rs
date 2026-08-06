use crate::check::core::EvaluationAnswer;
use crate::scope::q_scope_is_full_project;
use serde_json::{json, Value};

mod contract;
mod parse;

pub(crate) use contract::evaluator_response_output_schema_for_scope;
pub(crate) use parse::{
    matches_answer_pattern, parse_evaluator_response_for_short_id, EvaluatorResponseParseError,
};
#[cfg(test)]
pub(crate) use parse::{
    parse_evaluator_response_json, parse_evaluator_response_json_for_exact_requested_short_ids,
    UnvalidatedAgentResultJson,
};

pub(crate) const ERROR_SCOPE_TOO_NARROW: &str = "ScopeTooNarrow";
pub(crate) const ERROR_INVALID_QUESTION: &str = "InvalidQuestion";
pub(crate) const ANSWER_PATTERN: &str = "^[-_a-z0-9]+$";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvaluatorResponseSchemaScope {
    // An auto q-scope that currently hides files may be widened or narrowed.
    // Its schema therefore permits ScopeTooNarrow and requires a suggestion.
    AutoRestricted,
    // An auto q-scope at full project may still be narrowed, but it cannot be
    // widened. Its schema requires a suggestion but forbids ScopeTooNarrow.
    AutoFullProject,
    // A configured path-list q-scope is fixed for every turn. The canonical
    // fixed-scope exception forbids ScopeTooNarrow and omits suggestions even
    // when that path list is narrower than full project scope.
    FixedQScope,
    // When the evaluation never hides files, there is no scope to negotiate.
    // Its schema likewise forbids ScopeTooNarrow and omits suggestions.
    NoHiddenFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QScopeSuggestionSchemaPolicy {
    RequiredOnEveryAgentTurnResult,
    OmittedFromEveryAgentTurnResult,
}

const Q_SCOPE_SUGGESTION_MIN_ITEMS: usize = 1;
const Q_SCOPE_SUGGESTION_ITEM_MIN_LENGTH: usize = 1;
const Q_SCOPE_SUGGESTION_ITEM_PATTERN: &str = "^[^\\r\\n]*$";

impl QScopeSuggestionSchemaPolicy {
    fn requires_agent_q_scope_suggestion(self) -> bool {
        self == QScopeSuggestionSchemaPolicy::RequiredOnEveryAgentTurnResult
    }

    fn enforce_after_transport(self, suggestion: Option<&[String]>) -> Result<(), String> {
        // The serialized schema constrains structured output when the
        // transport supports each keyword. This local boundary deliberately
        // enforces the same policy again because parsed transport output is
        // untrusted and some transports approximate the selected schema.
        match (self, suggestion) {
            (QScopeSuggestionSchemaPolicy::OmittedFromEveryAgentTurnResult, Some(_)) => {
                Err("qScopeSuggestion must be omitted when no files are hidden".to_string())
            }
            (QScopeSuggestionSchemaPolicy::RequiredOnEveryAgentTurnResult, None) => {
                Err("qScopeSuggestion is required".to_string())
            }
            (QScopeSuggestionSchemaPolicy::RequiredOnEveryAgentTurnResult, Some(items)) => {
                if items.len() < Q_SCOPE_SUGGESTION_MIN_ITEMS {
                    return Err("qScopeSuggestion must contain at least one item".to_string());
                }
                if items.iter().any(|item| {
                    item.len() < Q_SCOPE_SUGGESTION_ITEM_MIN_LENGTH
                        || item.chars().any(|char| matches!(char, '\r' | '\n'))
                }) {
                    return Err(
                        "qScopeSuggestion items must be non-empty single-line strings".to_string(),
                    );
                }
                Ok(())
            }
            (QScopeSuggestionSchemaPolicy::OmittedFromEveryAgentTurnResult, None) => Ok(()),
        }
    }
}

impl EvaluatorResponseSchemaScope {
    pub(crate) fn for_auto_q_scope(q_scope: &[String]) -> EvaluatorResponseSchemaScope {
        // Interrogation Policy defines full project scope as exactly q-scope
        // ["."], before configured ignore exclusions are applied.
        if q_scope_is_full_project(q_scope) {
            EvaluatorResponseSchemaScope::AutoFullProject
        } else {
            EvaluatorResponseSchemaScope::AutoRestricted
        }
    }

    fn allowed_errors(self) -> &'static [&'static str] {
        // Evaluator response schemas only constrain evaluator-produced result
        // errors. Final `canon check` Error lines render CheckRecord errors,
        // which may also come from runtime or configuration failures.
        match self {
            EvaluatorResponseSchemaScope::AutoRestricted => {
                &[ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION]
            }
            EvaluatorResponseSchemaScope::AutoFullProject
            | EvaluatorResponseSchemaScope::FixedQScope
            | EvaluatorResponseSchemaScope::NoHiddenFiles => &[ERROR_INVALID_QUESTION],
        }
    }

    fn error_enum(self) -> Value {
        json!(self.allowed_errors())
    }

    fn allows_error(self, error: &str) -> bool {
        self.allowed_errors().contains(&error)
    }

    fn q_scope_suggestion_policy(self) -> QScopeSuggestionSchemaPolicy {
        // The response domain spans all four selected schemas, so its Rust
        // field is an Option. Each concrete schema is stricter: auto q-scope
        // responses require a suggestion, while fixed/no-hidden responses
        // must omit it.
        match self {
            EvaluatorResponseSchemaScope::AutoRestricted
            | EvaluatorResponseSchemaScope::AutoFullProject => {
                QScopeSuggestionSchemaPolicy::RequiredOnEveryAgentTurnResult
            }
            EvaluatorResponseSchemaScope::FixedQScope
            | EvaluatorResponseSchemaScope::NoHiddenFiles => {
                QScopeSuggestionSchemaPolicy::OmittedFromEveryAgentTurnResult
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) observed: String,
    pub(crate) error: Option<String>,
    // Supporting details are never a decision input. Evaluator answers provide
    // justification; internal errors may provide distinct technical diagnostics.
    pub(crate) evidence: Option<String>,
    pub(crate) scope: Vec<String>,
    pub(crate) q_scope_suggestion: Option<Vec<String>>,
}

impl ParsedAnswer {
    pub(crate) fn answer(
        answer: EvaluationAnswer,
        evidence: String,
        q_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            observed: answer.into_string(),
            error: None,
            evidence: Some(evidence),
            scope: Vec::new(),
            q_scope_suggestion,
        }
    }

    pub(crate) fn error_without_evidence(error: String) -> ParsedAnswer {
        // [Eg] Technical acquisition failures are constructed directly in the
        // evaluate domain. They are not agent JSON results and therefore do
        // not inherit the selected agent schema's qScopeSuggestion presence.
        ParsedAnswer {
            observed: error.clone(),
            error: Some(error),
            evidence: None,
            scope: Vec::new(),
            q_scope_suggestion: None,
        }
    }

    pub(crate) fn error_with_evidence(error: String, evidence: String) -> ParsedAnswer {
        // [Eg] Technical failures may carry diagnostics without becoming an
        // agent-produced answer/evidence response.
        ParsedAnswer {
            observed: error.clone(),
            error: Some(error),
            evidence: Some(evidence),
            scope: Vec::new(),
            q_scope_suggestion: None,
        }
    }
}
