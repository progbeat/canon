use crate::check::config::validation::validate_relative_config_path;
use crate::scope::normalize_repo_path;
use crate::scope::path_bytes_in_scope;
use std::path::Path;

pub(crate) fn expand_staged_foreach_paths_from_listing(
    config_path: &Path,
    glob: &str,
    staged_paths: &[Vec<u8>],
) -> Result<Vec<String>, String> {
    validate_relative_config_path(glob, "foreach path glob")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, glob))?;
    let foreach_pathspec = format!(":(glob){}", joined);
    let foreach_scope = std::slice::from_ref(&foreach_pathspec);
    let mut files = Vec::new();
    for staged_path in staged_paths {
        if path_bytes_in_scope(staged_path, foreach_scope)? {
            // [Mm,nK] Only a matched path must become the string `path`
            // binding. Unmatched source paths cannot narrow this glob.
            let path = std::str::from_utf8(staged_path).map_err(|_| {
                "!foreach matched a non-UTF-8 file path that cannot be bound to `path`".to_string()
            })?;
            files.push(path.to_string());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn join_repo_path(config_dir: &Path, glob: &str) -> String {
    if config_dir.as_os_str().is_empty() {
        glob.to_string()
    } else {
        format!(
            "{}/{}",
            config_dir.to_string_lossy().trim_end_matches('/'),
            glob
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: Mm
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

    #[test] // xpec: Mm
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

    #[test] // xpec: Mm
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

    #[test] // xpec: Mm,nK
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

    fn staged_paths(paths: &[&str]) -> Vec<Vec<u8>> {
        paths.iter().map(|path| path.as_bytes().to_vec()).collect()
    }
}
