use crate::scope::{path_bytes_in_scope, utf8_path_matches_glob};
use std::path::{Component, Path};

pub(crate) fn expand_staged_foreach_paths_from_listing(
    config_path: &Path,
    glob: &str,
    staged_paths: &[Vec<u8>],
) -> Result<Vec<String>, String> {
    let joined = resolve_foreach_path(config_path, glob, "foreach path glob")?;
    let foreach_pathspec = format!(":(glob){}", joined);
    let foreach_scope = std::slice::from_ref(&foreach_pathspec);
    let mut files = Vec::new();
    for staged_path in staged_paths {
        // [s6,nK] Valid UTF-8 paths use character glob semantics. Invalid
        // paths retain byte matching only to decide whether they would have
        // been selected before reporting that they cannot become a binding.
        let path = match std::str::from_utf8(staged_path) {
            Ok(path) if utf8_path_matches_glob(path, &joined) => path,
            Ok(_) => continue,
            Err(_) if path_bytes_in_scope(staged_path, foreach_scope)? => {
                return Err(
                    "!foreach matched a non-UTF-8 file path that cannot be bound to `path`"
                        .to_string(),
                );
            }
            Err(_) => continue,
        };
        files.push(path_relative_to_config(config_path, Path::new(path))?);
    }
    files.sort();
    Ok(files)
}

pub(crate) fn resolve_foreach_read_path(config_path: &Path, path: &str) -> Result<String, String> {
    resolve_foreach_path(config_path, path, "foreach read path")
}

fn resolve_foreach_path(config_path: &Path, path: &str, label: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err(format!("{label}: path must not be empty"));
    }
    if path.contains('\0') {
        return Err(format!("{label}: path must not contain NUL bytes"));
    }
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(format!("{label}: path must be relative: {path}"));
    }
    let mut parts = config_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("{label}: path must be valid UTF-8: {path}"))?;
                parts.push(part.to_string());
            }
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(format!("{label}: path escapes the source: {path}"));
                }
            }
            _ => return Err(format!("{label}: unsupported path component in {path}")),
        }
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

fn path_relative_to_config(config_path: &Path, path: &Path) -> Result<String, String> {
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    let path = path
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    let common = config_dir
        .iter()
        .zip(&path)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = vec!["..".to_string(); config_dir.len() - common];
    relative.extend(path[common..].iter().cloned());
    if relative.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(relative.join("/"))
    }
}

fn normal_component(component: Component<'_>) -> Option<String> {
    match component {
        Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: s6
    fn star_foreach_glob_matches_one_path_segment() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/child.txt",
                "src/root.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/root.md"]);
    }

    #[test] // xpec: s6
    fn double_star_foreach_glob_matches_nested_path_segments() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/**.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/nested/child.txt",
                "src/root.md",
            ]),
        )
        .unwrap();

        assert_eq!(
            files,
            vec![
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/root.md"
            ]
        );
    }

    #[test] // xpec: s6
    fn path_segment_foreach_globs_match_scope_pathspecs() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*/*.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/other/child.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/nested/child.md", "specs/other/child.md"]);
    }

    #[test] // xpec: s6
    fn question_mark_foreach_glob_matches_one_unicode_character() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/?.md",
            &staged_paths(&["specs/é.md", "specs/ab.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/é.md"]);
    }

    #[test] // xpec: s6
    fn foreach_glob_preserves_each_repeated_path_selection() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &staged_paths(&["specs/alpha.md", "specs/alpha.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/alpha.md", "specs/alpha.md"]);
    }

    #[test] // xpec: s6,nK
    fn non_utf8_paths_only_fail_when_the_glob_matches_them() {
        let unrelated_non_utf8 = vec![b'o', b't', b'h', b'e', b'r', b'/', 0xff];
        let files = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &[b"specs/good.md".to_vec(), unrelated_non_utf8],
        )
        .unwrap();

        assert_eq!(files, vec!["specs/good.md"]);

        let matched_non_utf8 = vec![
            b's', b'p', b'e', b'c', b's', b'/', b'f', 0xff, b'.', b'm', b'd',
        ];
        let error = expand_staged_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &[matched_non_utf8],
        )
        .unwrap_err();

        assert_eq!(
            error,
            "!foreach matched a non-UTF-8 file path that cannot be bound to `path`"
        );
    }

    #[test] // xpec: s6
    fn foreach_paths_stay_relative_to_the_document_directory() {
        let files = expand_staged_foreach_paths_from_listing(
            Path::new(".canon/includes/xpecs.yml"),
            "specs/*.md",
            &staged_paths(&[".canon/includes/specs/alpha.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/alpha.md"]);

        let files = expand_staged_foreach_paths_from_listing(
            Path::new(".canon/includes/xpecs.yml"),
            "../specs/*.md",
            &staged_paths(&[".canon/specs/root.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["../specs/root.md"]);
        assert_eq!(
            resolve_foreach_read_path(Path::new(".canon/includes/xpecs.yml"), "../specs/root.md")
                .unwrap(),
            ".canon/specs/root.md"
        );
    }

    fn staged_paths(paths: &[&str]) -> Vec<Vec<u8>> {
        paths.iter().map(|path| path.as_bytes().to_vec()).collect()
    }
}
