use crate::config_types::AgentConfig;
use crate::hash::full_scope;
use std::path::Path;

pub(crate) fn sanitize_scope(scope: &[String], agent: &AgentConfig) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope, Some(agent))
}

pub(crate) fn sanitize_scope_for_hash(scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope, None)
}

fn sanitize_scope_paths(
    scope: &[String],
    denied_agent: Option<&AgentConfig>,
) -> Result<Vec<String>, String> {
    if scope.is_empty() {
        return Err("scope must not be empty".to_string());
    }
    let mut normalized = Vec::new();
    let mut has_full_scope = false;
    for path in scope {
        let path = normalize_repo_path(path)?;
        if path != "." && denied_agent.is_some_and(|agent| is_denied_path(agent, &path)) {
            return Err(format!("scope path is denied: {}", path));
        }
        if path == "." {
            has_full_scope = true;
            continue;
        }
        normalized.push(path);
    }
    // The guard above rejects an originally empty scope. Reaching full scope
    // here requires an explicit "." entry or an internal caller that normalized
    // a current-directory spelling to "." before canonicalization.
    if has_full_scope || normalized.is_empty() {
        Ok(full_scope())
    } else {
        Ok(canonicalize_scope_paths(normalized))
    }
}

pub(crate) fn canonicalize_scope_paths(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    let mut canonical: Vec<String> = Vec::new();
    for path in paths {
        if canonical.iter().any(|parent| scope_contains(parent, &path)) {
            continue;
        }
        canonical.push(path);
    }
    if canonical.is_empty() {
        full_scope()
    } else {
        canonical
    }
}

pub(crate) fn normalize_repo_path(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("path must not be empty".to_string());
    }
    // Git paths may contain newlines and other control bytes, and scope
    // hashing length-prefixes every entry before hashing. NUL is different:
    // Git paths and process arguments cannot represent it, so reject it at the
    // normalized repo-path boundary instead of failing later in Command::arg.
    if value.contains('\0') {
        return Err("path must not contain NUL bytes".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!("path must be relative: {}", value));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("path must be valid UTF-8: {}", value))?;
                parts.push(part.to_string());
            }
            std::path::Component::ParentDir => {
                return Err(format!("path must not contain '..': {}", value));
            }
            _ => return Err(format!("unsupported path component in {}", value)),
        }
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

pub(crate) fn is_denied_path(agent: &AgentConfig, path: &str) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern(path, pattern))
}

#[cfg(test)]
pub(crate) fn is_denied_path_bytes(agent: &AgentConfig, path: &[u8]) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern_bytes(path, pattern.as_bytes()))
}

pub(crate) fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    path_matches_pattern_bytes(path.as_bytes(), pattern.as_bytes())
}

pub(crate) fn path_matches_pattern_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let path = trim_dot_slash_bytes(path);
    let pattern = trim_dot_slash_bytes(pattern);
    if path == pattern {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(b"/**") {
        return glob_path_matches_bytes(path, prefix) || glob_prefix_matches_path(path, prefix);
    }
    glob_path_matches_bytes(path, pattern)
}

fn glob_prefix_matches_path(path: &[u8], prefix: &[u8]) -> bool {
    path.iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'/' && glob_path_matches_bytes(&path[..index], prefix))
}

fn glob_path_matches_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let mut matches = vec![false; path.len() + 1];
    matches[0] = true;
    for pattern_byte in pattern {
        let mut next = vec![false; path.len() + 1];
        for index in 0..=path.len() {
            if !matches[index] {
                continue;
            }
            match *pattern_byte {
                b'*' => {
                    next[index] = true;
                    let mut end = index;
                    while end < path.len() && path[end] != b'/' {
                        end += 1;
                        next[end] = true;
                    }
                }
                b'?' if index < path.len() && path[index] != b'/' => {
                    next[index + 1] = true;
                }
                literal if index < path.len() && path[index] == literal => {
                    next[index + 1] = true;
                }
                _ => {}
            }
        }
        matches = next;
    }
    matches[path.len()]
}

fn trim_dot_slash_bytes(mut path: &[u8]) -> &[u8] {
    while path.starts_with(b"./") {
        path = &path[2..];
    }
    path
}

pub(crate) fn is_strict_scope_subset(proposed: &[String], current: &[String]) -> bool {
    let proposed = canonicalize_scope_paths(proposed.to_vec());
    let current = canonicalize_scope_paths(current.to_vec());
    if proposed == current {
        return false;
    }
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

pub(crate) fn scope_is_within(proposed: &[String], current: &[String]) -> bool {
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

pub(crate) fn scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = MANDATORY_EVALUATOR_DENY_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    for pattern in &agent.ignore {
        let pattern = normalized_ignore_pattern(pattern);
        if !patterns.iter().any(|existing| existing == &pattern) {
            patterns.push(pattern);
        }
    }
    patterns
}

pub(crate) fn normalized_ignore_pattern(pattern: &str) -> String {
    normalize_repo_path(pattern)
        .unwrap_or_else(|_| pattern.strip_prefix("./").unwrap_or(pattern).to_string())
}

pub(crate) const MANDATORY_EVALUATOR_DENY_PATTERNS: &[&str] = &[
    ".canon",
    ".canon/**",
    ".git/canon",
    ".git/canon/**",
    ".git/canon/logs",
    ".git/canon/logs/**",
];
