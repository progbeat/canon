use crate::check_types::SelectedExpectation;
use crate::config_types::AgentConfig;
use crate::hash::hash_120;
use crate::scope::effective_ignore_patterns;

pub(crate) fn history_cache_key(agent: &AgentConfig, expectation: &SelectedExpectation) -> String {
    // History directories stay keyed by q/a for stable cleanup semantics.
    // `cacheKey` is emitted for audit/debugging only; reusable cache selection
    // remains governed by the history record's scope and current scopeTreeOid.
    let mut input = Vec::new();
    push_history_cache_key_part(&mut input, "schema", "2");
    push_history_cache_key_part(&mut input, "instructions", agent.custom_instructions());
    let mut ignore_patterns = effective_ignore_patterns(agent);
    ignore_patterns.sort();
    for pattern in ignore_patterns {
        push_history_cache_key_part(&mut input, "ignore", &pattern);
    }
    for plugin in &agent.plugins {
        push_history_cache_key_part(&mut input, "plugin", plugin);
    }
    if let Some(primary) = &agent.model.primary {
        push_history_cache_key_part(&mut input, "model.primary", primary);
    }
    for fallback in &agent.model.fallbacks {
        push_history_cache_key_part(&mut input, "model.fallback", fallback);
    }
    let thinking = expectation
        .thinking
        .as_deref()
        .unwrap_or(agent.thinking.as_str());
    push_history_cache_key_part(&mut input, "thinking", thinking);
    let cooldown = expectation
        .cooldown
        .map(|cooldown| cooldown.seconds.to_string())
        .unwrap_or_else(|| "none".to_string());
    push_history_cache_key_part(&mut input, "cooldown", &cooldown);
    hash_120(&input)
}

fn push_history_cache_key_part(input: &mut Vec<u8>, key: &str, value: &str) {
    input.extend_from_slice(key.as_bytes());
    input.push(0);
    input.extend_from_slice(value.len().to_string().as_bytes());
    input.push(0);
    input.extend_from_slice(value.as_bytes());
    input.push(0);
}
