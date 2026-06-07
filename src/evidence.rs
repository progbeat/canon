use crate::scope::{normalize_repo_path, path_bytes_in_scope};
use std::path::Path;

pub(crate) fn evidence_file_refs_are_visible(evidence: &str, visible_scope: &[String]) -> bool {
    evidence_file_refs_are_visible_in_root(evidence, visible_scope, None)
}

pub(crate) fn evidence_file_refs_are_visible_in_root(
    evidence: &str,
    visible_scope: &[String],
    root: Option<&Path>,
) -> bool {
    backtick_refs(evidence).all(|reference| {
        let Some(path) = project_file_ref_path(reference) else {
            return true;
        };
        path_bytes_in_scope(path.as_bytes(), visible_scope).unwrap_or(false)
            && root.is_none_or(|root| root.join(&path).exists())
    })
}

fn backtick_refs(text: &str) -> BacktickRefs<'_> {
    BacktickRefs { rest: text }
}

struct BacktickRefs<'a> {
    rest: &'a str,
}

impl<'a> Iterator for BacktickRefs<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.rest.find('`')?;
        let after_start = &self.rest[start + 1..];
        let end = after_start.find('`')?;
        let reference = &after_start[..end];
        self.rest = &after_start[end + 1..];
        Some(reference)
    }
}

fn project_file_ref_path(reference: &str) -> Option<String> {
    let path = strip_line_suffix(reference.trim());
    if !looks_like_project_file_path(path) {
        return None;
    }
    normalize_repo_path(path).ok()
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

fn looks_like_project_file_path(path: &str) -> bool {
    if path.is_empty()
        || path.ends_with('/')
        || path.starts_with(':')
        || path.bytes().any(|byte| {
            matches!(byte, b'*' | b'?' | b'[' | b']') || byte.is_ascii_whitespace() || byte == b'\0'
        })
    {
        return false;
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.starts_with("canon/") {
        return false;
    }
    if path.contains('/') {
        return true;
    }
    matches!(path, "Dockerfile" | "Makefile" | "LICENSE")
        || path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| file_extension_is_common_project_file(extension))
}

fn file_extension_is_common_project_file(extension: &str) -> bool {
    matches!(
        extension,
        "css"
            | "html"
            | "js"
            | "json"
            | "jsx"
            | "lock"
            | "md"
            | "rs"
            | "sh"
            | "toml"
            | "ts"
            | "tsx"
            | "txt"
            | "yaml"
            | "yml"
    )
}

#[cfg(test)]
mod tests {
    use super::{evidence_file_refs_are_visible, evidence_file_refs_are_visible_in_root};
    use std::fs;

    #[test]
    fn file_refs_must_be_inside_visible_scope() {
        let visible_scope = vec!["src/evaluator/config.rs".to_string()];

        assert!(!evidence_file_refs_are_visible(
            "`src/platform/platform_unix.rs:126-700` mixes responsibilities.",
            &visible_scope,
        ));
    }

    #[test]
    fn full_scope_accepts_project_file_refs() {
        assert!(evidence_file_refs_are_visible(
            "`src/platform/platform_unix.rs:126-700` mixes responsibilities.",
            &[".".to_string()],
        ));
    }

    #[test]
    fn configured_exclusions_hide_matching_refs() {
        let visible_scope = vec![".".to_string(), ":(exclude,glob).canon/**".to_string()];

        assert!(!evidence_file_refs_are_visible(
            "` .canon/check.yml ` is hidden.",
            &visible_scope,
        ));
    }

    #[test]
    fn code_identifiers_and_pathspecs_are_not_file_refs() {
        let visible_scope = vec!["src".to_string()];

        assert!(evidence_file_refs_are_visible(
            "`qScopeSuggestion`, `record.scope`, `canon gate`, `canon/logs`, `.canon/**`, `.canon/`, `:(exclude,glob).canon/**`, and `:10-20` are not file refs.",
            &visible_scope,
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
            Some(&root),
        ));
        assert!(!evidence_file_refs_are_visible_in_root(
            "`src/deleted.rs` does not exist.",
            &[".".to_string()],
            Some(&root),
        ));

        let _ = fs::remove_dir_all(root);
    }
}
