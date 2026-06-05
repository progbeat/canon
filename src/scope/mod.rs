use crate::config_types::AgentConfig;
use crate::hash::full_scope;
use std::collections::HashMap;
use std::path::Path;

// Keep this implementation map in sync with `docs/GLOSSARY.md`: this module
// owns scope path normalization, Git pathspec matching, and ignore matching.
// Visible-scope selection lives in `check::interrogation::state`;
// visible-tree hashing/materialization live in `git::visible_tree_oid` and
// `staged::worktree`; q-scope verification/storage/reuse live in
// `check::interrogation::policy`, `history::store`, and `history::reuse`;
// evaluator-thread reuse invariants live in `check::interrogation::state` and
// `check::interrogation::thread`.

pub(crate) fn sanitize_scope(
    scope: &[String],
    _agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope)
}

pub(crate) fn sanitize_scope_for_hash(scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope)
}

pub(crate) fn visible_scope(
    agent: &AgentConfig,
    q_scope: &[String],
) -> Result<Vec<String>, String> {
    let mut scope = sanitize_scope_paths(q_scope)?;
    for pattern in effective_ignore_patterns(agent)? {
        scope.push(excluding_pathspec(&pattern));
    }
    Ok(scope)
}

fn sanitize_scope_paths(scope: &[String]) -> Result<Vec<String>, String> {
    if scope.is_empty() {
        return Err("scope must not be empty".to_string());
    }
    let mut normalized = Vec::new();
    let mut has_full_scope = false;
    for path in scope {
        let path = normalize_repo_path(path)?;
        // Scope normalization keeps the q-scope as a Git pathspec list. The
        // visible scope is formed later by appending configured exclusions.
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

pub(crate) fn path_bytes_in_scope(path: &[u8], scope: &[String]) -> Result<bool, String> {
    if scope.is_empty() {
        return Err("scope must not be empty".to_string());
    }
    let mut has_include = false;
    let mut included = false;
    for pathspec in scope {
        if pathspec_is_exclude(pathspec)? {
            continue;
        }
        has_include = true;
        if path_bytes_match_scope_base(path, pathspec)? {
            included = true;
        }
    }
    if !has_include {
        included = true;
    }
    for pathspec in scope {
        if pathspec_is_exclude(pathspec)? && path_bytes_match_scope_base(path, pathspec)? {
            included = false;
        }
    }
    Ok(included)
}

fn path_bytes_match_scope_base(path: &[u8], base: &str) -> Result<bool, String> {
    let Some(pattern) = pathspec_magic_pattern(base)? else {
        let base = trim_dot_slash_bytes(base.as_bytes());
        return Ok(normalized_scope_contains_bytes(
            base,
            trim_dot_slash_bytes(path),
        ));
    };
    Ok(match pattern.magic {
        PathspecMagic::Glob => path_matches_pathspec_glob_bytes(path, pattern.path.as_bytes()),
        PathspecMagic::Literal => {
            normalized_scope_contains_bytes(pattern.path.as_bytes(), trim_dot_slash_bytes(path))
        }
    })
}

struct PathspecPattern<'a> {
    magic: PathspecMagic,
    exclude: bool,
    path: &'a str,
}

enum PathspecMagic {
    Glob,
    Literal,
}

fn pathspec_magic_pattern(pathspec: &str) -> Result<Option<PathspecPattern<'_>>, String> {
    if let Some(path) = pathspec.strip_prefix(":!") {
        return Ok(Some(PathspecPattern {
            magic: PathspecMagic::Glob,
            exclude: true,
            path,
        }));
    }
    if let Some(path) = pathspec.strip_prefix(":^") {
        return Ok(Some(PathspecPattern {
            magic: PathspecMagic::Glob,
            exclude: true,
            path,
        }));
    }
    let Some(rest) = pathspec.strip_prefix(":(") else {
        return Ok(None);
    };
    let end = rest
        .find(')')
        .ok_or_else(|| format!("unsupported pathspec magic: {}", pathspec))?;
    let magic_text = &rest[..end];
    let path = &rest[end + 1..];
    let mut exclude = false;
    let mut match_magic = None;
    for magic in magic_text.split(',') {
        match magic {
            "exclude" => exclude = true,
            "glob" => match_magic = Some(PathspecMagic::Glob),
            "literal" => match_magic = Some(PathspecMagic::Literal),
            "" => {}
            _ => return Err(format!("unsupported pathspec magic: {}", magic)),
        }
    }
    Ok(Some(PathspecPattern {
        magic: match_magic.unwrap_or(PathspecMagic::Literal),
        exclude,
        path,
    }))
}

pub(crate) fn pathspec_is_exclude(pathspec: &str) -> Result<bool, String> {
    Ok(pathspec_magic_pattern(pathspec)?.is_some_and(|pattern| pattern.exclude))
}

fn normalized_scope_contains_bytes(base: &[u8], path: &[u8]) -> bool {
    base == b"." || path == base || path.starts_with(&slash_terminated_base(base))
}

fn slash_terminated_base(base: &[u8]) -> Vec<u8> {
    let mut prefix = base.to_vec();
    prefix.push(b'/');
    prefix
}

fn path_matches_pathspec_glob_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let path = trim_dot_slash_bytes(path);
    let pattern = trim_dot_slash_bytes(pattern);
    pathspec_glob_matches_at(path, pattern, 0, 0, &mut HashMap::new())
}

fn pathspec_glob_matches_at(
    path: &[u8],
    pattern: &[u8],
    path_index: usize,
    pattern_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(result) = memo.get(&(path_index, pattern_index)) {
        return *result;
    }
    let result = if pattern_index == pattern.len() {
        path_index == path.len()
    } else if pattern[pattern_index..].starts_with(b"**/") {
        double_star_slash_matches(path, pattern, path_index, pattern_index, memo)
    } else if pattern[pattern_index..].starts_with(b"**") {
        double_star_matches(path, pattern, path_index, pattern_index, memo)
    } else {
        match pattern[pattern_index] {
            b'*' => star_matches(path, pattern, path_index, pattern_index, memo),
            b'?' if path.get(path_index).is_some_and(|byte| *byte != b'/') => {
                pathspec_glob_matches_at(path, pattern, path_index + 1, pattern_index + 1, memo)
            }
            literal if path.get(path_index).is_some_and(|byte| *byte == literal) => {
                pathspec_glob_matches_at(path, pattern, path_index + 1, pattern_index + 1, memo)
            }
            _ => false,
        }
    };
    memo.insert((path_index, pattern_index), result);
    result
}

fn double_star_slash_matches(
    path: &[u8],
    pattern: &[u8],
    path_index: usize,
    pattern_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    let next_pattern = pattern_index + 3;
    if pathspec_glob_matches_at(path, pattern, path_index, next_pattern, memo) {
        return true;
    }
    for index in path_index..path.len() {
        if path[index] == b'/'
            && pathspec_glob_matches_at(path, pattern, index + 1, next_pattern, memo)
        {
            return true;
        }
    }
    false
}

fn double_star_matches(
    path: &[u8],
    pattern: &[u8],
    path_index: usize,
    pattern_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    let next_pattern = pattern_index + 2;
    (path_index..=path.len())
        .any(|index| pathspec_glob_matches_at(path, pattern, index, next_pattern, memo))
}

fn star_matches(
    path: &[u8],
    pattern: &[u8],
    path_index: usize,
    pattern_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    let next_pattern = pattern_index + 1;
    if pathspec_glob_matches_at(path, pattern, path_index, next_pattern, memo) {
        return true;
    }
    let mut index = path_index;
    while index < path.len() && path[index] != b'/' {
        index += 1;
        if pathspec_glob_matches_at(path, pattern, index, next_pattern, memo) {
            return true;
        }
    }
    false
}

fn trim_dot_slash_bytes(mut path: &[u8]) -> &[u8] {
    while path.starts_with(b"./") {
        path = &path[2..];
    }
    path
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

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Result<Vec<String>, String> {
    let mut patterns = Vec::new();
    for pattern in &agent.ignore {
        let pattern = normalized_ignore_pattern(pattern)?;
        push_unique_pattern(&mut patterns, pattern);
    }
    Ok(patterns)
}

fn excluding_pathspec(pattern: &str) -> String {
    format!(":(exclude,glob){}", pattern)
}

fn push_unique_pattern(patterns: &mut Vec<String>, pattern: String) {
    if !patterns.iter().any(|existing| existing == &pattern) {
        patterns.push(pattern);
    }
}

pub(crate) fn normalized_ignore_pattern(pattern: &str) -> Result<String, String> {
    normalize_repo_path(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_scope_applies_configured_ignores_as_pathspec_exclusions() {
        let agent = AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: vec![".canon/**".to_string()],
            plugins: Vec::new(),
        };
        let scope = visible_scope(&agent, &full_scope()).unwrap();

        assert_eq!(
            scope,
            vec![".".to_string(), ":(exclude,glob).canon/**".to_string()]
        );
        assert!(!path_bytes_in_scope(b".canon/TODOs.md", &scope).unwrap());
        assert!(path_bytes_in_scope(b"src/main.rs", &scope).unwrap());
    }

    #[test]
    fn plain_scope_paths_are_literal_even_with_wildcard_bytes() {
        let scope = vec!["src/*".to_string()];

        assert!(path_bytes_in_scope(b"src/*", &scope).unwrap());
        assert!(path_bytes_in_scope(b"src/*/literal-child.rs", &scope).unwrap());
        assert!(!path_bytes_in_scope(b"src/main.rs", &scope).unwrap());
        assert!(!path_bytes_in_scope(b"src/nested/main.rs", &scope).unwrap());
    }

    #[test]
    fn explicit_glob_pathspecs_use_path_segment_wildcards() {
        let one_segment = vec![":(glob)src/*".to_string()];
        let recursive = vec![":(glob)src/**".to_string()];
        let middle_recursive = vec![":(glob)foo/**/bar".to_string()];

        assert!(path_bytes_in_scope(b"src/main.rs", &one_segment).unwrap());
        assert!(!path_bytes_in_scope(b"src/nested/main.rs", &one_segment).unwrap());
        assert!(path_bytes_in_scope(b"src/nested/main.rs", &recursive).unwrap());
        assert!(path_bytes_in_scope(b"foo/bar", &middle_recursive).unwrap());
        assert!(path_bytes_in_scope(b"foo/a/b/bar", &middle_recursive).unwrap());
    }

    #[test]
    fn invalid_scope_pathspecs_are_not_treated_as_fallback_matches() {
        assert!(path_bytes_in_scope(b"src/main.rs", &[]).is_err());
        assert!(path_bytes_in_scope(b"src/main.rs", &[":(icase)src/main.rs".to_string()]).is_err());
    }
}
