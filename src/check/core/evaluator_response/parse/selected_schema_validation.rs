use super::super::{
    EvaluationAnswer, EvaluatorResponseSchemaScope, ParsedAnswer, QScopeSuggestionSchemaPolicy,
    ANSWER_PATTERN,
};
use super::matches_answer_pattern;
use serde::{de, Deserialize};
use serde_json::value::RawValue;
use serde_json::Value;
use std::collections::{btree_map::Entry, BTreeMap};

/// Unvalidated JSON candidate emitted by an agent turn for one short ID.
///
/// A parsed candidate is not yet an evaluation response. Only
/// `into_schema_valid_parsed_answer` crosses that boundary, after every member
/// has satisfied the selected Interrogation Policy schema.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnvalidatedAgentResultJson {
    #[serde(
        default,
        rename = "answer",
        deserialize_with = "deserialize_optional_answer_candidate"
    )]
    pub(crate) unvalidated_answer: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_error")]
    pub(crate) error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_evidence")]
    pub(crate) evidence: Option<String>,
    #[serde(
        default,
        rename = "qScopeSuggestion",
        deserialize_with = "deserialize_optional_q_scope_suggestion"
    )]
    // Raw Option covers every selected schema plus malformed input. It is not
    // the evaluate-domain optionality from the pseudocode; schema validation
    // resolves it to `ValidatedAgentQScopeSuggestionPresence` first.
    pub(crate) unvalidated_q_scope_suggestion: Option<Vec<String>>,
}

enum ValidatedAgentQScopeSuggestionPresence {
    RequiredInEveryAgentTurnResult(Vec<String>),
    OmittedFromEveryAgentTurnResult,
}

impl ValidatedAgentQScopeSuggestionPresence {
    fn into_evaluate_optional(self) -> Option<Vec<String>> {
        // [Eg,qv] `evaluate` spans response modes, so its pseudocode consumes
        // an optional suggestion. Optionality starts only here, after the
        // selected auto schema required presence on answer and error alike.
        match self {
            ValidatedAgentQScopeSuggestionPresence::RequiredInEveryAgentTurnResult(suggestion) => {
                Some(suggestion)
            }
            ValidatedAgentQScopeSuggestionPresence::OmittedFromEveryAgentTurnResult => None,
        }
    }
}

pub(super) struct AgentTurnEnvelopeJson {
    pub(super) responses: BTreeMap<String, Box<RawValue>>,
    pub(super) duplicate_short_id: Option<String>,
}

impl<'de> Deserialize<'de> for AgentTurnEnvelopeJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct ResponsesVisitor;

        impl<'de> de::Visitor<'de> for ResponsesVisitor {
            type Value = AgentTurnEnvelopeJson;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an evaluator response object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut responses = BTreeMap::new();
                let mut duplicate_short_id = None;
                while let Some(short_id) = map.next_key::<String>()? {
                    let response = map.next_value::<Box<RawValue>>()?;
                    match responses.entry(short_id) {
                        Entry::Vacant(entry) => {
                            entry.insert(response);
                        }
                        Entry::Occupied(entry) => {
                            duplicate_short_id.get_or_insert_with(|| entry.key().clone());
                        }
                    }
                }
                Ok(AgentTurnEnvelopeJson {
                    responses,
                    duplicate_short_id,
                })
            }
        }

        deserializer.deserialize_map(ResponsesVisitor)
    }
}

impl UnvalidatedAgentResultJson {
    pub(super) fn into_schema_valid_parsed_answer(
        self,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<ParsedAnswer, String> {
        self.validate_schema(schema_scope)?;
        let validated_q_scope_suggestion = match schema_scope.q_scope_suggestion_policy() {
            QScopeSuggestionSchemaPolicy::RequiredOnEveryAgentTurnResult => {
                ValidatedAgentQScopeSuggestionPresence::RequiredInEveryAgentTurnResult(
                    self.unvalidated_q_scope_suggestion
                        .expect("selected auto schema validation requires qScopeSuggestion"),
                )
            }
            QScopeSuggestionSchemaPolicy::OmittedFromEveryAgentTurnResult => {
                ValidatedAgentQScopeSuggestionPresence::OmittedFromEveryAgentTurnResult
            }
        };
        let q_scope_suggestion = validated_q_scope_suggestion.into_evaluate_optional();
        if let Some(answer) = self.unvalidated_answer {
            // Pass/fail comparison happens after parsing against the
            // expectation's current expected answer. The domain type is
            // constructed only after the selected schema accepted the raw
            // JSON string above.
            return Ok(ParsedAnswer::answer(
                EvaluationAnswer::new(answer),
                self.evidence
                    .expect("schema validation ensures answer evidence is present"),
                q_scope_suggestion,
            ));
        }
        let error = self
            .error
            .expect("schema validation ensures error is present");
        // [Eg,qv] This is an error emitted inside a successful agent turn, so
        // the selected auto schema requires qScopeSuggestion. Technical
        // exceptions caught by evaluate never pass through this parser and
        // construct ParsedAnswer without a suggestion instead.
        Ok(ParsedAnswer {
            observed: error.clone(),
            error: Some(error),
            evidence: None,
            scope: Vec::new(),
            q_scope_suggestion,
        })
    }

    pub(crate) fn validate_schema(
        &self,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<(), String> {
        let has_answer = self.unvalidated_answer.is_some();
        let has_error = self.error.is_some();
        if has_answer == has_error {
            return Err(
                "evaluator response must contain exactly one of answer or error".to_string(),
            );
        }
        if has_answer {
            let Some(_) = self.evidence.as_deref() else {
                return Err("evidence is required with answer".to_string());
            };
        }
        if has_error && self.evidence.is_some() {
            return Err("evidence must be omitted with error".to_string());
        }
        if let Some(answer) = self.unvalidated_answer.as_deref() {
            if !matches_answer_pattern(answer) {
                return Err(format!("answer must match pattern {}", ANSWER_PATTERN));
            }
        }
        if let Some(error) = self.error.as_deref() {
            if !schema_scope.allows_error(error) {
                return Err(format!("unsupported evaluator error: {}", error));
            }
        }
        // [qv] The selected policy owns both the declarative transport schema
        // and this mandatory post-transport enforcement. These are distinct
        // trust boundaries, not independent copies of the response contract.
        schema_scope
            .q_scope_suggestion_policy()
            .enforce_after_transport(self.unvalidated_q_scope_suggestion.as_deref())?;
        Ok(())
    }
}

fn deserialize_optional_answer_candidate<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    struct SelectedSchemaAnswerCandidateVisitor;

    impl de::Visitor<'_> for SelectedSchemaAnswerCandidateVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // [MH,qv] An agent turn emits the response itself, so its selected
            // schema requires this member to have reached the string domain.
            // Producer-native scalars such as shell exit codes are normalized
            // by their producer before response construction.
            formatter.write_str(
                "a JSON string answer required by the selected agent response schema; \
                 producer scalar normalization occurs before response construction",
            )
        }

        fn visit_str<E>(self, answer: &str) -> Result<Self::Value, E> {
            Ok(answer.to_string())
        }

        fn visit_string<E>(self, answer: String) -> Result<Self::Value, E> {
            Ok(answer)
        }
    }

    deserializer
        .deserialize_string(SelectedSchemaAnswerCandidateVisitor)
        .map(Some)
}

fn deserialize_optional_error<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_schema_string_field(deserializer, "error")
}

fn deserialize_optional_evidence<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_schema_string_field(deserializer, "evidence")
}

fn deserialize_optional_schema_string_field<'de, D>(
    deserializer: D,
    field_name: &'static str,
) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom(format!(
            "{} must not be null",
            field_name
        )));
    }
    String::deserialize(value)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_optional_q_scope_suggestion<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom("qScopeSuggestion must not be null"));
    }
    Vec::<String>::deserialize(value)
        .map(Some)
        .map_err(de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::super::super::{EvaluatorResponseSchemaScope, ParsedAnswer, ERROR_INVALID_QUESTION};
    use super::super::EvaluatorResponseParseError;
    use super::*;

    fn keyed_response(result_json: &str) -> String {
        format!(r#"{{"q":{}}}"#, result_json)
    }

    fn parse_evaluator_response(
        result_json: &str,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
        super::super::parse_evaluator_response_for_short_id(
            &keyed_response(result_json),
            schema_scope,
            "q",
            &[],
        )
    }

    fn parse_evaluator_response_json(
        result_json: &str,
    ) -> Result<UnvalidatedAgentResultJson, EvaluatorResponseParseError> {
        super::super::parse_evaluator_response_json(&keyed_response(result_json), "q", &[])
    }

    #[test] // xpec: qv
    fn auto_full_project_evaluator_response_rejects_scope_too_narrow() {
        let error = parse_evaluator_response(
            r#"{"error":"ScopeTooNarrow","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::AutoFullProject,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ScopeTooNarrow"));
    }

    #[test] // xpec: qv
    fn evaluator_error_omits_evidence() {
        let response = parse_evaluator_response(
            r#"{"error":"InvalidQuestion","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
        )
        .unwrap();

        assert_eq!(response.error.as_deref(), Some(ERROR_INVALID_QUESTION));
        assert_eq!(response.evidence, None);
    }

    #[test] // xpec: qv
    fn evaluator_error_rejects_evidence() {
        let error = parse_evaluator_response(
            r#"{"error":"InvalidQuestion","evidence":"details","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
        )
        .unwrap_err();

        assert!(error.to_string().contains("evidence must be omitted"));
    }

    #[test] // xpec: qv
    fn evaluator_answer_requires_evidence() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
        )
        .unwrap_err();

        assert!(error.to_string().contains("evidence is required"));
    }

    #[test] // xpec: qv
    fn non_string_json_never_crosses_the_selected_agent_response_boundary() {
        for answer_candidate in ["7", "true", "[]", "{}"] {
            let result_json = format!(
                r#"{{"answer":{answer_candidate},"evidence":"details","qScopeSuggestion":["."]}}"#
            );
            let error = parse_evaluator_response(
                &result_json,
                EvaluatorResponseSchemaScope::AutoRestricted,
            )
            .unwrap_err();

            assert!(error.to_string().contains("string"));
        }
    }

    #[test] // xpec: qv
    fn evaluator_evidence_accepts_any_string() {
        for evidence in [
            "",
            "   ",
            "`src/check.rs:...` claims a violation.",
            "Implementation details are in `src/check.rs ...`.",
            "unclosed `code",
        ] {
            let response = serde_json::json!({"answer": "yes", "evidence": evidence}).to_string();
            let answer =
                parse_evaluator_response(&response, EvaluatorResponseSchemaScope::FixedQScope)
                    .unwrap();

            assert_eq!(answer.evidence.as_deref(), Some(evidence));
        }
    }

    #[test] // xpec: qv
    fn auto_restricted_evaluator_response_requires_q_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs:1`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: qv
    fn auto_agent_error_result_requires_q_scope_suggestion() {
        let response = parse_evaluator_response_json(r#"{"error":"InvalidQuestion"}"#).unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: qv
    fn auto_full_project_evaluator_response_requires_q_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs:1`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::AutoFullProject)
            .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_null_fields() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","error":null,"evidence":"`src/main.rs:1`","qScopeSuggestion":null}"#,
            EvaluatorResponseSchemaScope::AutoFullProject,
        )
        .unwrap_err();

        assert!(error.to_string().contains("must not be null"));
    }

    #[test] // xpec: qv
    fn fixed_q_scope_and_no_hidden_files_responses_omit_q_scope_suggestion() {
        for schema_scope in [
            EvaluatorResponseSchemaScope::FixedQScope,
            EvaluatorResponseSchemaScope::NoHiddenFiles,
        ] {
            let response = parse_evaluator_response(
                r#"{"answer":"yes","evidence":"`src/main.rs:1`"}"#,
                schema_scope,
            )
            .unwrap();

            assert_eq!(response.observed, "yes");
            assert_eq!(response.q_scope_suggestion, None);
        }
    }

    #[test] // xpec: qv
    fn auto_full_project_evaluator_response_accepts_q_scope_suggestion() {
        let response = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["src/main.rs"]}"#,
            EvaluatorResponseSchemaScope::AutoFullProject,
        )
        .unwrap();

        assert_eq!(
            response.q_scope_suggestion,
            Some(vec!["src/main.rs".to_string()])
        );
    }

    #[test] // xpec: qv
    fn fixed_q_scope_and_no_hidden_files_responses_reject_q_scope_suggestion() {
        for schema_scope in [
            EvaluatorResponseSchemaScope::FixedQScope,
            EvaluatorResponseSchemaScope::NoHiddenFiles,
        ] {
            let error = parse_evaluator_response(
                r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]}"#,
                schema_scope,
            )
            .unwrap_err();

            assert!(error.to_string().contains("qScopeSuggestion"));
        }
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_empty_q_scope_suggestion() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":[]}"#,
        )
        .unwrap();

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_empty_q_scope_suggestion_item() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":[""]}"#,
        )
        .unwrap();

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err()
            .contains("non-empty"));
    }

    #[test] // xpec: qv
    fn evaluator_response_accepts_required_q_scope_suggestion() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["src/main.rs"]}"#,
        )
        .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap();
        assert_eq!(
            response.unvalidated_q_scope_suggestion,
            Some(vec!["src/main.rs".to_string()])
        );
    }

    #[test] // xpec: qv
    fn evaluator_response_schema_allows_non_crlf_q_scope_chars() {
        let response = parse_evaluator_response_json(
            "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs:1`\",\"qScopeSuggestion\":[\"src/main.rs\\u0008\"]}",
        )
        .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap();
    }

    #[test] // xpec: qv
    fn evaluator_response_schema_rejects_answers_outside_answer_pattern() {
        for invalid_answer in ["Rust", "yes\t", "yes\n", "yes\u{2028}still", ""] {
            let response = UnvalidatedAgentResultJson {
                unvalidated_answer: Some(invalid_answer.to_string()),
                error: None,
                evidence: Some("ok".to_string()),
                unvalidated_q_scope_suggestion: Some(vec![".".to_string()]),
            };

            assert!(response
                .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
                .unwrap_err()
                .contains("answer"));
        }
    }

    #[test] // xpec: qv
    fn evaluator_response_schema_rejects_non_canonical_error_token() {
        let response = UnvalidatedAgentResultJson {
            unvalidated_answer: None,
            error: Some("TechnicalFailure".to_string()),
            evidence: None,
            unvalidated_q_scope_suggestion: Some(vec![".".to_string()]),
        };

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err()
            .contains("unsupported evaluator error"));
    }

    #[test] // xpec: qv
    fn evaluator_response_schema_rejects_crlf_in_single_line_fields() {
        let answer = parse_evaluator_response_json(
            "{\"answer\":\"yes\\n\",\"evidence\":\"`src/main.rs:1`\",\"qScopeSuggestion\":[\"src/main.rs\"]}",
        )
        .unwrap();
        let q_scope = parse_evaluator_response_json(
            "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs:1`\",\"qScopeSuggestion\":[\"src\\rmain.rs\"]}",
        )
        .unwrap();

        assert!(answer
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err()
            .contains("answer"));
        assert!(q_scope
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test] // xpec: qv
    fn evaluator_response_schema_rejects_only_crlf_q_scope_line_breaks() {
        let schema_valid_unicode_separator = UnvalidatedAgentResultJson {
            unvalidated_answer: Some("yes".to_string()),
            error: None,
            evidence: Some("ok".to_string()),
            unvalidated_q_scope_suggestion: Some(vec!["src\u{2028}main.rs".to_string()]),
        };
        assert!(schema_valid_unicode_separator
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .is_ok());

        let schema_invalid_crlf = UnvalidatedAgentResultJson {
            unvalidated_answer: Some("yes".to_string()),
            error: None,
            evidence: Some("ok".to_string()),
            unvalidated_q_scope_suggestion: Some(vec!["src\nmain.rs".to_string()]),
        };
        assert!(schema_invalid_crlf
            .validate_schema(EvaluatorResponseSchemaScope::AutoRestricted)
            .is_err());
    }
}
