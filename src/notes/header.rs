use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(crate) fn validate_note_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    if key.chars().any(char::is_control) {
        return Err("key must not contain control characters".to_string());
    }
    Ok(())
}

pub(crate) fn initial_content(key: &str, hash: &str) -> String {
    header(key, hash)
}

pub(crate) fn header(key: &str, hash: &str) -> String {
    format!(
        "<!-- canon key=\"{}\" hash=\"{}\" -->\n# {}\n",
        escape_attr(key),
        hash,
        key
    )
}

pub(crate) fn normalize_body(text: &str) -> String {
    let mut value = text.to_string();
    while value.ends_with('\n') {
        value.pop();
    }
    value
}

pub(crate) fn verify_note_key(path: &Path, expected_key: &str) -> Result<(), String> {
    let first = first_line(path)?;
    verify_note_key_from_first_line(path, &first, expected_key)
}

pub(crate) fn verify_note_key_from_first_line(
    path: &Path,
    first: &str,
    expected_key: &str,
) -> Result<(), String> {
    let actual_key = parse_key_from_header(first)
        .ok_or_else(|| format!("missing canon metadata in {}", path.display()))?;
    if actual_key != expected_key {
        return Err(format!(
            "hash collision or stale file: {} belongs to key {:?}, not {:?}",
            path.display(),
            actual_key,
            expected_key
        ));
    }
    Ok(())
}

pub(crate) fn first_line(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

pub(crate) fn parse_key_from_header(line: &str) -> Option<String> {
    let prefix = "<!-- canon key=\"";
    let rest = line.strip_prefix(prefix)?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => {
                let escaped = chars.next()?;
                match escaped {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    _ => return None,
                }
            }
            other => out.push(other),
        }
    }
    None
}

pub(crate) fn escape_attr(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}
