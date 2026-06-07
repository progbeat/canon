use crate::hash::full_scope;
use std::path::Path;

pub(crate) fn sanitize_scope(scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope)
}

pub(super) fn sanitize_scope_paths(scope: &[String]) -> Result<Vec<String>, String> {
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

fn canonicalize_scope_paths(mut paths: Vec<String>) -> Vec<String> {
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

fn scope_contains(base: &str, path: &str) -> bool {
    let Ok(base) = normalize_repo_path(base) else {
        return false;
    };
    let Ok(path) = normalize_repo_path(path) else {
        return false;
    };
    normalized_scope_contains(&base, &path)
}

pub(super) fn normalized_scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

pub(super) fn normalize_scope_for_comparison(scope: &[String]) -> Option<Vec<String>> {
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
