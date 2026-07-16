use crate::check::config::validation::validate_relative_config_path;
use crate::scope::normalize_repo_path;
use crate::scope::path_bytes_in_scope;
use std::path::Path;

pub(crate) fn expand_staged_foreach_paths_from_listing(
    config_path: &Path,
    glob: &str,
    staged_paths: &[String],
) -> Result<Vec<String>, String> {
    validate_relative_config_path(glob, "foreach path glob")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, glob))?;
    let foreach_pathspec = format!(":(glob){}", joined);
    let foreach_scope = std::slice::from_ref(&foreach_pathspec);
    let mut files = Vec::new();
    for staged_path in staged_paths {
        if path_bytes_in_scope(staged_path.as_bytes(), foreach_scope)? {
            files.push(staged_path.clone());
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

    fn staged_paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }
}
