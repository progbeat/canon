use std::path::Path;

const OUTPUT_LIMIT_BYTES: usize = 16 * 1024;
const TRUNCATION_MARKER: &str = "[truncated]\n";
const OUTPUT_CONTENT_LIMIT_BYTES: usize = OUTPUT_LIMIT_BYTES - TRUNCATION_MARKER.len();

pub(super) fn project_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn push_bounded_line(output: &mut String, line: &str) -> bool {
    if output.len().saturating_add(line.len()).saturating_add(1) > OUTPUT_CONTENT_LIMIT_BYTES {
        return false;
    }
    output.push_str(line);
    output.push('\n');
    true
}

pub(super) fn finish_bounded_output(mut output: String, truncated: bool) -> Result<String, String> {
    if truncated {
        if output.len() > OUTPUT_CONTENT_LIMIT_BYTES {
            return Err("project inspection output exceeded its bounded content limit".to_string());
        }
        output.push_str(TRUNCATION_MARKER);
    } else if output.len() > OUTPUT_LIMIT_BYTES {
        return Err("project inspection output exceeded its bounded output limit".to_string());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: qv,hQ
    fn truncation_marker_stays_inside_the_output_bound() {
        let mut output = String::new();
        let fitting_line = "x".repeat(OUTPUT_CONTENT_LIMIT_BYTES - 1);

        assert!(push_bounded_line(&mut output, &fitting_line));
        assert!(!push_bounded_line(&mut output, "x"));
        let finished = finish_bounded_output(output, true).unwrap();

        assert!(finished.len() <= OUTPUT_LIMIT_BYTES);
        assert!(finished.ends_with(TRUNCATION_MARKER));
        assert!(finish_bounded_output("x".repeat(OUTPUT_LIMIT_BYTES + 1), false).is_err());
    }
}
