use crate::check_types::ParsedAnswer;
use crate::config_types::AgentConfig;
use crate::evaluator_response::parse_evaluator_response;
use crate::scope::effective_ignore_patterns;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct EvaluatorResponseParseCache {
    values: BTreeMap<(String, Vec<String>), Result<ParsedAnswer, String>>,
}

impl EvaluatorResponseParseCache {
    pub(crate) fn new() -> EvaluatorResponseParseCache {
        EvaluatorResponseParseCache::default()
    }

    pub(crate) fn parse(
        &mut self,
        text: &str,
        agent: &AgentConfig,
    ) -> Result<ParsedAnswer, String> {
        let key = (text.to_string(), effective_ignore_patterns(agent));
        if let Some(parsed) = self.values.get(&key) {
            return parsed.clone();
        }
        let parsed = parse_evaluator_response(text, agent);
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
