use crate::check::{
    parse_evaluator_response_for_short_id, EvaluatorResponseParseError,
    EvaluatorResponseSchemaScope, ParsedAnswer,
};
use crate::config_types::AgentConfig;
use crate::scope::effective_ignore_patterns;
use std::collections::BTreeMap;

type ParseCacheKey = (
    String,
    Vec<String>,
    EvaluatorResponseSchemaScope,
    String,
    Vec<String>,
);
type ParseCacheValue = Result<ParsedAnswer, EvaluatorResponseParseError>;

#[derive(Default)]
pub(crate) struct EvaluatorResponseParseCache {
    values: BTreeMap<ParseCacheKey, ParseCacheValue>,
}

impl EvaluatorResponseParseCache {
    pub(crate) fn new() -> EvaluatorResponseParseCache {
        EvaluatorResponseParseCache::default()
    }

    pub(crate) fn parse(
        &mut self,
        text: &str,
        agent: &AgentConfig,
        schema_scope: EvaluatorResponseSchemaScope,
        short_id: &str,
        answered_short_ids: &[String],
    ) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
        let key = (
            text.to_string(),
            effective_ignore_patterns(agent).map_err(EvaluatorResponseParseError::Schema)?,
            schema_scope,
            short_id.to_string(),
            answered_short_ids.to_vec(),
        );
        if let Some(parsed) = self.values.get(&key) {
            return parsed.clone();
        }
        let parsed =
            parse_evaluator_response_for_short_id(text, schema_scope, short_id, answered_short_ids);
        self.values.insert(key, parsed.clone());
        parsed
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
