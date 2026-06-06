use crate::check::config::validation::validate_relative_config_path;
use crate::scope::normalize_repo_path;
use std::path::Path;

pub(crate) fn expand_staged_generator_paths_from_listing(
    config_path: &Path,
    path: &str,
    staged_paths: &[String],
) -> Result<Vec<String>, String> {
    validate_relative_config_path(path, "expectation generator path")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, path))?;
    let mut files = staged_paths
        .iter()
        .filter(|staged_path| generator_pattern_matches(&joined, staged_path))
        .cloned()
        .collect::<Vec<_>>();
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

fn generator_pattern_matches(pattern: &str, path: &str) -> bool {
    let Some(star_index) = pattern.find('*') else {
        return path == pattern;
    };
    let slash_index = pattern[..star_index]
        .rfind('/')
        .map(|index| index + 1)
        .unwrap_or(0);
    let dir = &pattern[..slash_index].trim_end_matches('/');
    let file_pattern = &pattern[slash_index..];
    let expected_prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{}/", dir)
    };
    let Some(file_name) = path.strip_prefix(&expected_prefix) else {
        return false;
    };
    !file_name.contains('/') && wildcard_match(file_pattern, file_name)
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut remaining = text;
    if let Some(prefix) = parts.first().filter(|prefix| !prefix.is_empty()) {
        let Some(stripped) = remaining.strip_prefix(prefix) else {
            return false;
        };
        remaining = stripped;
    }
    let middle_end = parts.len().saturating_sub(1);
    for part in &parts[1..middle_end] {
        if part.is_empty() {
            continue;
        }
        let Some(index) = remaining.find(part) else {
            return false;
        };
        remaining = &remaining[index + part.len()..];
    }
    if let Some(suffix) = parts.last().filter(|suffix| !suffix.is_empty()) {
        remaining.ends_with(suffix)
    } else {
        true
    }
}
