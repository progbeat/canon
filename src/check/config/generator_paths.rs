use crate::check::config::validation::validate_relative_config_path;
use crate::scope::normalize_repo_path;
use crate::scope::path_bytes_in_scope;
use std::path::Path;

pub(crate) fn expand_staged_generator_paths_from_listing(
    config_path: &Path,
    path: &str,
    staged_paths: &[String],
) -> Result<Vec<String>, String> {
    validate_relative_config_path(path, "expectation generator path")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, path))?;
    let generator_pathspec = format!(":(glob){}", joined);
    let generator_scope = std::slice::from_ref(&generator_pathspec);
    let mut files = Vec::new();
    for staged_path in staged_paths {
        if path_bytes_in_scope(staged_path.as_bytes(), generator_scope)? {
            files.push(staged_path.clone());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn join_repo_path(config_dir: &Path, path: &str) -> String {
    if config_dir.as_os_str().is_empty() {
        path.to_string()
    } else {
        format!(
            "{}/{}",
            config_dir.to_string_lossy().trim_end_matches('/'),
            path
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_generator_path_matches_one_path_segment() {
        let files = expand_staged_generator_paths_from_listing(
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

    #[test]
    fn double_star_generator_path_matches_nested_path_segments() {
        let files = expand_staged_generator_paths_from_listing(
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

    #[test]
    fn path_segment_generator_globs_match_like_scope_pathspecs() {
        let files = expand_staged_generator_paths_from_listing(
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

    fn staged_paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }
}
