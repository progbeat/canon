use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::path_bytes_in_scope;

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
