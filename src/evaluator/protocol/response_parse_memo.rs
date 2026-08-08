use crate::check::{
    parse_evaluator_response_for_short_id, EvaluatorResponseParseError,
    EvaluatorResponseSchemaScope, ParsedAnswer,
};
use std::collections::{btree_map::Entry, BTreeMap};

// [d,fh] This memo is invocation-local, lives only in ThreadState memory, and
// is dropped with that state; it is not persistent response or xpec storage.
// Its key matches the parser arguments exactly so unrelated evaluator
// configuration cannot split one deterministic parse result into distinct
// entries.
type ResponseParseMemoKey = (String, EvaluatorResponseSchemaScope, String, Vec<String>);
type ResponseParseMemoValue = Result<ParsedAnswer, EvaluatorResponseParseError>;

#[derive(Default)]
pub(crate) struct InvocationResponseParseMemo {
    values: BTreeMap<ResponseParseMemoKey, ResponseParseMemoValue>,
}

impl InvocationResponseParseMemo {
    pub(crate) fn new() -> InvocationResponseParseMemo {
        InvocationResponseParseMemo::default()
    }

    pub(crate) fn parse(
        &mut self,
        text: &str,
        schema_scope: EvaluatorResponseSchemaScope,
        short_id: &str,
        answered_short_ids: &[String],
    ) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
        let key = (
            text.to_string(),
            schema_scope,
            short_id.to_string(),
            answered_short_ids.to_vec(),
        );
        match self.values.entry(key) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let parsed = parse_evaluator_response_for_short_id(
                    text,
                    schema_scope,
                    short_id,
                    answered_short_ids,
                );
                entry.insert(parsed.clone());
                parsed
            }
        }
    }
}

pub(crate) fn response_excerpt(text: &str) -> String {
    const LIMIT: usize = 600;
    let text = text.trim();
    if text.is_empty() {
        return "<empty>".to_string();
    }
    let mut excerpt = text.chars().take(LIMIT).collect::<String>();
    if text.chars().count() > LIMIT {
        excerpt.push_str("...");
    }
    excerpt
}
