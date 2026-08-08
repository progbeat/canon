pub(crate) use crate::check::core::escape_inline_text as escape_check_output_text;
use crate::json_util::compact_json_string_array;

pub(crate) fn push_escaped_check_output_line(output: &mut String, label: &str, value: &str) {
    output.push_str(label);
    output.push_str(": ");
    output.push_str(&escape_check_output_text(value));
    output.push('\n');
}

pub(crate) fn push_error_and_evidence_lines(
    output: &mut String,
    error: &str,
    evidence: Option<&str>,
) {
    output.push_str("error: ");
    output.push_str(error);
    output.push('\n');
    if let Some(evidence) = evidence {
        push_escaped_check_output_line(output, "evidence", evidence);
    }
}

pub(crate) fn push_observed_and_evidence_lines(
    output: &mut String,
    observed: &str,
    evidence: Option<&str>,
) {
    push_escaped_check_output_line(output, "observed", observed);
    if let Some(evidence) = evidence {
        push_escaped_check_output_line(output, "evidence", evidence);
    }
}

pub(crate) fn push_diff_from_line(
    output: &mut String,
    diff_from: &str,
    diff_from_tree_oid_abbrev: &str,
) {
    output.push_str("diff-from: ");
    output.push_str(diff_from_tree_oid_abbrev);
    output.push_str(" (");
    output.push_str(diff_from);
    output.push_str(")\n");
}

pub(crate) fn push_q_scope_suggestion(output: &mut String, suggestion: &[String]) {
    output.push_str("q-scope-suggestion: ");
    output.push_str(&compact_json_string_array(suggestion));
    output.push('\n');
}
