use super::normalize::normalize_repo_path;
use crate::config_types::AgentConfig;

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Result<Vec<String>, String> {
    let mut patterns = Vec::new();
    if let Some(ignore) = &agent.ignore {
        for pattern in ignore {
            let pattern = normalized_ignore_pattern(pattern)?;
            push_unique_pattern(&mut patterns, pattern);
        }
    }
    Ok(patterns)
}

pub(super) fn effective_ignore_exclusion_pathspecs(
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    Ok(effective_ignore_patterns(agent)?
        .into_iter()
        .map(|pattern| excluding_pathspec(&pattern))
        .collect())
}

pub(super) fn push_unique_pattern(patterns: &mut Vec<String>, pattern: String) {
    if !patterns.iter().any(|existing| existing == &pattern) {
        patterns.push(pattern);
    }
}

fn normalized_ignore_pattern(pattern: &str) -> Result<String, String> {
    normalize_repo_path(pattern)
}

fn excluding_pathspec(pattern: &str) -> String {
    format!(":(exclude,glob){}", pattern)
}
