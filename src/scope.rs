use crate::config_types::AgentConfig;
use crate::hash::full_scope;
use std::path::Path;

// Glossary implementation map: this module owns scope path normalization,
// Git pathspec matching, and ignore matching. Visible-scope selection lives in
// `check_interrogation_state`; visible-tree hashing/materialization live in
// `git_runtime::visible_tree_oid` and `staged_snapshot::staged_worktree`;
// q-scope verification/storage/reuse live in `checking::check`,
// `check_interrogation_policy`, `history_store::history`, and
// `history_store::history_reuse`; evaluator-thread reuse invariants live in
// `check_interrogation_state` and `check_interrogation`.

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
        // "." is the logical full project scope, not permission to read every
        // physical subtree. Mandatory and agent ignore patterns are still
        // enforced later when building evaluator permissions and visible scope
        // hashes, so rejecting "." here would make full-scope checks
        // impossible without increasing evaluator access.
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

pub(crate) fn is_denied_path_bytes(agent: &AgentConfig, path: &[u8]) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern_bytes(path, pattern.as_bytes()))
}

pub(crate) fn path_bytes_in_scope(path: &[u8], scope: &[String]) -> bool {
    if scope == full_scope() {
        return true;
    }
    scope
        .iter()
        .any(|base| path_bytes_match_scope_base(path, base))
}

pub(crate) fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    let Ok(path) = normalize_repo_path(path) else {
        return false;
    };
    let Ok(pattern) = normalize_repo_path(pattern) else {
        return false;
    };
    path_matches_normalized_pattern(&path, &pattern)
}

fn path_matches_normalized_pattern(path: &str, pattern: &str) -> bool {
    if path == pattern {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return glob_path_matches(path, prefix) || glob_prefix_matches_path(path, prefix);
    }
    glob_path_matches(path, pattern)
}

pub(crate) fn path_matches_pattern_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let path = trim_dot_slash_bytes(path);
    let pattern = trim_dot_slash_bytes(pattern);
    if path == pattern {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(b"/**") {
        return glob_path_matches_bytes(path, prefix)
            || glob_prefix_matches_path_bytes(path, prefix);
    }
    glob_path_matches_bytes(path, pattern)
}

fn path_bytes_match_scope_base(path: &[u8], base: &str) -> bool {
    let Some(pattern) = pathspec_magic_pattern(base) else {
        let base = trim_dot_slash_bytes(base.as_bytes());
        if pathspec_has_wildcard_bytes(base) {
            return path_matches_pattern_bytes(path, base);
        }
        return normalized_scope_contains_bytes(base, trim_dot_slash_bytes(path));
    };
    match pattern.magic {
        PathspecMagic::Glob => path_matches_pathspec_glob_bytes(path, pattern.path.as_bytes()),
        PathspecMagic::Literal => {
            normalized_scope_contains_bytes(pattern.path.as_bytes(), trim_dot_slash_bytes(path))
        }
    }
}

struct PathspecPattern<'a> {
    magic: PathspecMagic,
    path: &'a str,
}

enum PathspecMagic {
    Glob,
    Literal,
}

fn pathspec_magic_pattern(pathspec: &str) -> Option<PathspecPattern<'_>> {
    let rest = pathspec.strip_prefix(":(")?;
    let end = rest.find(')')?;
    let magic = &rest[..end];
    let path = &rest[end + 1..];
    if magic.split(',').any(|item| item == "glob") {
        return Some(PathspecPattern {
            magic: PathspecMagic::Glob,
            path,
        });
    }
    if magic.split(',').any(|item| item == "literal") {
        return Some(PathspecPattern {
            magic: PathspecMagic::Literal,
            path,
        });
    }
    None
}

fn pathspec_has_wildcard_bytes(pathspec: &[u8]) -> bool {
    pathspec.iter().any(|byte| matches!(*byte, b'*' | b'?'))
}

fn normalized_scope_contains_bytes(base: &[u8], path: &[u8]) -> bool {
    base == b"." || path == base || path.starts_with(&slash_terminated_base(base))
}

fn slash_terminated_base(base: &[u8]) -> Vec<u8> {
    let mut prefix = base.to_vec();
    prefix.push(b'/');
    prefix
}

fn glob_prefix_matches_path(path: &str, prefix: &str) -> bool {
    path.match_indices('/')
        .any(|(index, _)| glob_path_matches(&path[..index], prefix))
}

fn glob_path_matches(path: &str, pattern: &str) -> bool {
    let path = path.chars().collect::<Vec<_>>();
    let pattern = pattern.chars().collect::<Vec<_>>();
    let mut matches = vec![false; path.len() + 1];
    matches[0] = true;
    for pattern_char in pattern {
        let mut next = vec![false; path.len() + 1];
        for index in 0..=path.len() {
            if !matches[index] {
                continue;
            }
            match pattern_char {
                '*' => {
                    next[index] = true;
                    let mut end = index;
                    while end < path.len() {
                        end += 1;
                        next[end] = true;
                    }
                }
                '?' if index < path.len() => {
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

fn glob_prefix_matches_path_bytes(path: &[u8], prefix: &[u8]) -> bool {
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
                    while end < path.len() {
                        end += 1;
                        next[end] = true;
                    }
                }
                b'?' if index < path.len() => {
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

fn path_matches_pathspec_glob_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let path = trim_dot_slash_bytes(path);
    let pattern = trim_dot_slash_bytes(pattern);
    let mut matches = vec![false; path.len() + 1];
    matches[0] = true;
    let mut pattern_index = 0;
    while pattern_index < pattern.len() {
        let mut next = vec![false; path.len() + 1];
        let pattern_byte = pattern[pattern_index];
        let double_star = pattern_byte == b'*'
            && pattern
                .get(pattern_index + 1)
                .is_some_and(|byte| *byte == b'*');
        for index in 0..=path.len() {
            if !matches[index] {
                continue;
            }
            match pattern_byte {
                b'*' if double_star => {
                    next[index] = true;
                    let mut end = index;
                    while end < path.len() {
                        end += 1;
                        next[end] = true;
                    }
                }
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
        pattern_index += if double_star { 2 } else { 1 };
    }
    matches[path.len()]
}

fn trim_dot_slash_bytes(mut path: &[u8]) -> &[u8] {
    while path.starts_with(b"./") {
        path = &path[2..];
    }
    path
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_strict_scope_subset(proposed: &[String], current: &[String]) -> bool {
    let Some(proposed) = normalize_scope_for_comparison(proposed) else {
        return false;
    };
    let Some(current) = normalize_scope_for_comparison(current) else {
        return false;
    };
    if proposed == current {
        return false;
    }
    proposed.iter().all(|path| {
        current
            .iter()
            .any(|base| normalized_scope_contains(base, path))
    })
}

pub(crate) fn scope_is_within(proposed: &[String], current: &[String]) -> bool {
    let Some(proposed) = normalize_scope_for_comparison(proposed) else {
        return false;
    };
    let Some(current) = normalize_scope_for_comparison(current) else {
        return false;
    };
    proposed.iter().all(|path| {
        current
            .iter()
            .any(|base| normalized_scope_contains(base, path))
    })
}

pub(crate) fn scope_contains(base: &str, path: &str) -> bool {
    let Ok(base) = normalize_repo_path(base) else {
        return false;
    };
    let Ok(path) = normalize_repo_path(path) else {
        return false;
    };
    normalized_scope_contains(&base, &path)
}

fn normalized_scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

fn normalize_scope_for_comparison(scope: &[String]) -> Option<Vec<String>> {
    if scope.is_empty() {
        return None;
    }
    let mut normalized = Vec::new();
    let mut has_full_scope = false;
    for path in scope {
        let path = normalize_repo_path(path).ok()?;
        if path == "." {
            has_full_scope = true;
            continue;
        }
        normalized.push(path);
    }
    if has_full_scope || normalized.is_empty() {
        Some(full_scope())
    } else {
        Some(canonicalize_scope_paths(normalized))
    }
}

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = MANDATORY_EVALUATOR_DENY_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    for pattern in &agent.ignore {
        match normalized_ignore_pattern(pattern) {
            Ok(pattern) => push_unique_pattern(&mut patterns, pattern),
            Err(_) => {
                for fallback in INVALID_IGNORE_PATTERN_FAIL_CLOSED_PATTERNS {
                    push_unique_pattern(&mut patterns, (*fallback).to_string());
                }
            }
        }
    }
    patterns
}

fn push_unique_pattern(patterns: &mut Vec<String>, pattern: String) {
    if !patterns.iter().any(|existing| existing == &pattern) {
        patterns.push(pattern);
    }
}

pub(crate) fn normalized_ignore_pattern(pattern: &str) -> Result<String, String> {
    normalize_repo_path(pattern)
}

pub(crate) const MANDATORY_EVALUATOR_DENY_PATTERNS: &[&str] = &[
    ".canon",
    ".canon/**",
    ".git/canon",
    ".git/canon/**",
    ".git/canon/logs",
    ".git/canon/logs/**",
];

const INVALID_IGNORE_PATTERN_FAIL_CLOSED_PATTERNS: &[&str] = &["*", "*/**"];
