use crate::check_validation::validate_relative_config_path;
use crate::scope::normalize_repo_path;
use std::fs;
use std::path::Path;

pub(crate) fn expand_generator_paths(
    root: &Path,
    config_path: &Path,
    path: &str,
    staged: bool,
) -> Result<Vec<String>, String> {
    validate_relative_config_path(path, "expectation generator path")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, path))?;
    if staged {
        return Err("staged generator expansion requires RepoInspectionCache".to_string());
    }
    expand_filesystem_generator_paths(root, &joined)
}

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

pub(crate) fn join_repo_path(config_dir: &Path, path: &str) -> String {
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

pub(crate) fn expand_filesystem_generator_paths(
    root: &Path,
    pattern: &str,
) -> Result<Vec<String>, String> {
    let Some(star_index) = pattern.find('*') else {
        let path = root.join(pattern);
        return if path.is_file() {
            Ok(vec![pattern.to_string()])
        } else {
            Ok(Vec::new())
        };
    };
    let slash_index = pattern[..star_index]
        .rfind('/')
        .map(|index| index + 1)
        .unwrap_or(0);
    let dir = &pattern[..slash_index].trim_end_matches('/');
    let file_pattern = &pattern[slash_index..];
    let dir_path = if dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(dir)
    };
    if !dir_path.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir_path)
        .map_err(|err| format!("failed to read {}: {}", dir_path.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", dir_path.display(), err))?;
        let file_name = entry.file_name();
        let file_name_for_match = file_name.to_string_lossy();
        if wildcard_match(file_pattern, &file_name_for_match) && entry.path().is_file() {
            let file_name = file_name
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 spec path in {}", dir_path.display()))?;
            let repo_path = if dir.is_empty() {
                file_name.to_string()
            } else {
                format!("{}/{}", dir, file_name)
            };
            files.push(repo_path);
        }
    }
    files.sort();
    Ok(files)
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

pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
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
