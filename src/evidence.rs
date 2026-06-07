use crate::scope::{normalize_repo_path, path_bytes_in_scope};
use std::path::Path;

pub(crate) fn evidence_file_refs_are_visible_in_root(
    evidence: &str,
    visible_scope: &[String],
    root: &Path,
) -> bool {
    let mut search_start = 0;
    while let Some(start_offset) = evidence[search_start..].find('`') {
        let start = search_start + start_offset;
        let after_start = start + 1;
        let Some(end) = evidence[after_start..]
            .find('`')
            .map(|offset| after_start + offset)
        else {
            search_start = after_start;
            continue;
        };

        let Some(path) = project_file_ref_path(&evidence[after_start..end], root) else {
            search_start = end;
            continue;
        };
        if !path_bytes_in_scope(path.as_bytes(), visible_scope).unwrap_or(false)
            || !root.join(&path).is_file()
        {
            return false;
        }
        search_start = end + 1;
    }
    true
}

fn project_file_ref_path(reference: &str, root: &Path) -> Option<String> {
    let reference = reference.trim();
    if let Some(path) = existing_project_path(reference, root) {
        return Some(path);
    }
    let path = strip_line_suffix(reference);
    project_file_ref_path_without_line_suffix(path, root)
}

fn existing_project_path(reference: &str, root: &Path) -> Option<String> {
    let path = normalize_candidate_project_file_path(reference, root)?;
    root.join(&path).exists().then_some(path)
}

fn project_file_ref_path_without_line_suffix(reference: &str, root: &Path) -> Option<String> {
    let path = normalize_candidate_project_file_path(reference, root)?;
    if path.contains('/') {
        return Some(path);
    }
    if root.join(&path).exists() {
        Some(path)
    } else {
        None
    }
}

fn strip_line_suffix(reference: &str) -> &str {
    let Some((path, suffix)) = reference.rsplit_once(':') else {
        return reference;
    };
    if !path.is_empty() && location_suffix_is_lines(suffix) {
        path
    } else {
        reference
    }
}

fn location_suffix_is_lines(suffix: &str) -> bool {
    !suffix.is_empty() && suffix.split(',').all(line_part_is_digits_or_range)
}

fn line_part_is_digits_or_range(part: &str) -> bool {
    let Some((start, end)) = part.split_once('-') else {
        return !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    };
    !start.is_empty()
        && !end.is_empty()
        && start.bytes().all(|byte| byte.is_ascii_digit())
        && end.bytes().all(|byte| byte.is_ascii_digit())
}

fn normalize_candidate_project_file_path(path: &str, root: &Path) -> Option<String> {
    if path.is_empty()
        || path.ends_with('/')
        || path.starts_with(':')
        || path
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']') || byte == b'\0')
    {
        return None;
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.starts_with("canon/") {
        return None;
    }
    let path = normalize_repo_path(path).ok()?;
    if path.bytes().any(|byte| byte.is_ascii_whitespace()) {
        if root.join(&path).exists() {
            return Some(path);
        }
        if !missing_whitespace_path_ref_has_file_extension(&path) {
            return None;
        }
    }
    Some(path)
}

fn missing_whitespace_path_ref_has_file_extension(path: &str) -> bool {
    path.contains('/')
        && path
            .rsplit('/')
            .next()
            .is_some_and(file_name_has_plain_extension)
}

fn file_name_has_plain_extension(file_name: &str) -> bool {
    let Some((stem, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !extension.is_empty()
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::evidence_file_refs_are_visible_in_root;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn file_refs_must_be_inside_visible_scope() {
        let root = temp_root("scope");
        fs::create_dir_all(root.join("src/platform")).unwrap();
        fs::write(root.join("src/platform/platform_unix.rs"), "unix\n").unwrap();
        let visible_scope = vec!["src/evaluator/config.rs".to_string()];

        assert!(!evidence_file_refs_are_visible_in_root(
            "`src/platform/platform_unix.rs:126-700` mixes responsibilities.",
            &visible_scope,
            &root,
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stray_backtick_does_not_hide_later_file_refs() {
        let root = temp_root("stray");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/visible.rs"), "visible\n").unwrap();
        fs::write(root.join("src/hidden.rs"), "hidden\n").unwrap();
        let visible_scope = vec!["src/visible.rs".to_string()];

        assert!(!evidence_file_refs_are_visible_in_root(
            "`src/visible.rs` is visible, stray ` text `src/hidden.rs` is hidden.",
            &visible_scope,
            &root,
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adjacent_file_refs_are_checked_independently() {
        let root = temp_root("adjacent");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/visible.rs"), "visible\n").unwrap();
        fs::write(root.join("src/hidden.rs"), "hidden\n").unwrap();
        let visible_scope = vec!["src/visible.rs".to_string()];

        assert!(!evidence_file_refs_are_visible_in_root(
            "`src/visible.rs``src/hidden.rs`",
            &visible_scope,
            &root,
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn full_scope_accepts_project_file_refs() {
        assert!(evidence_file_refs_are_visible_in_root(
            "`src/platform/platform_unix.rs:126-700` mixes responsibilities.",
            &[".".to_string()],
            repo_root(),
        ));
    }

    #[test]
    fn configured_exclusions_hide_matching_refs() {
        let visible_scope = vec![".".to_string(), ":(exclude,glob).canon/**".to_string()];

        assert!(!evidence_file_refs_are_visible_in_root(
            "` .canon/check.yml ` is hidden.",
            &visible_scope,
            repo_root(),
        ));
    }

    #[test]
    fn code_identifiers_and_pathspecs_are_not_file_refs() {
        let root = temp_root("identifiers");
        fs::create_dir_all(root.join("src")).unwrap();
        let visible_scope = vec!["src".to_string()];

        assert!(evidence_file_refs_are_visible_in_root(
            "`qScopeSuggestion`, `record.scope`, `canon gate`, `canon/logs`, `.canon/**`, `.canon/`, `:(exclude,glob).canon/**`, and `:10-20` are not file refs.",
            &visible_scope,
            &root,
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn root_file_refs_are_checked_against_scope() {
        let visible_scope = vec!["src".to_string()];

        assert!(!evidence_file_refs_are_visible_in_root(
            "`Cargo.toml` is outside scope.",
            &visible_scope,
            repo_root(),
        ));
    }

    #[test]
    fn file_refs_must_exist_when_root_is_available() {
        let root = std::env::temp_dir().join(format!("canon-evidence-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        assert!(evidence_file_refs_are_visible_in_root(
            "`src/main.rs` exists.",
            &[".".to_string()],
            &root,
        ));
        assert!(evidence_file_refs_are_visible_in_root(
            "`src/main.rs:1` strips line syntax.",
            &[".".to_string()],
            &root,
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`src/deleted.rs` does not exist.",
            &[".".to_string()],
            &root,
        ));
        fs::write(root.join("CustomBuild"), "rule\n").unwrap();
        assert!(evidence_file_refs_are_visible_in_root(
            "`CustomBuild` exists.",
            &[".".to_string()],
            &root,
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`CustomBuild` is outside scope.",
            &["src".to_string()],
            &root,
        ));
        fs::create_dir_all(root.join("CustomDirectory")).unwrap();
        assert!(!evidence_file_refs_are_visible_in_root(
            "`CustomDirectory` is not a file.",
            &[".".to_string()],
            &root,
        ));
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/colon:123"), "notes\n").unwrap();
        fs::write(root.join("docs/My File.md"), "notes\n").unwrap();
        fs::write(root.join("docs/Release Notes"), "notes\n").unwrap();
        fs::create_dir_all(root.join("docs/Release Notes Directory")).unwrap();
        assert!(evidence_file_refs_are_visible_in_root(
            "`docs/colon:123` keeps literal file names.",
            &[".".to_string()],
            &root,
        ));
        assert!(evidence_file_refs_are_visible_in_root(
            "`docs/colon:123:7` strips line syntax after checking the literal path.",
            &[".".to_string()],
            &root,
        ));
        assert!(evidence_file_refs_are_visible_in_root(
            "`docs/My File.md` exists.",
            &[".".to_string()],
            &root,
        ));
        assert!(evidence_file_refs_are_visible_in_root(
            "`docs/Release Notes` exists.",
            &[".".to_string()],
            &root,
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`docs/My File.md` is outside scope.",
            &["src".to_string()],
            &root,
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`docs/Release Notes Directory` is not a file.",
            &[".".to_string()],
            &root,
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`docs/Missing File.md` does not exist.",
            &["src".to_string()],
            &root,
        ));

        let _ = fs::remove_dir_all(root);
    }

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canon-evidence-test-{}-{}",
            label,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
