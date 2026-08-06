use crate::check::command::output::{escape_check_output_text, write_stdout_record};
use crate::check::ResolvedExpectation;
use std::io;

pub(crate) fn write_show_expectations(expectations: &[ResolvedExpectation]) -> Result<(), String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_show_expectations_to(&mut stdout, expectations)
}

pub(super) fn write_show_expectations_to(
    output: &mut dyn std::io::Write,
    expectations: &[ResolvedExpectation],
) -> Result<(), String> {
    for expectation in expectations {
        write_stdout_record(
            output,
            render_canonical_show_record(expectation).as_bytes(),
            "show expectation",
        )?;
    }
    Ok(())
}

pub(super) fn render_show_expectations_text(expectations: &[ResolvedExpectation]) -> String {
    expectations
        .iter()
        .map(render_canonical_show_record)
        .collect::<String>()
}

pub(super) fn render_canonical_show_record(expectation: &ResolvedExpectation) -> String {
    // [2gZ] This single renderer serves both `canon show` and dynamic
    // `canon.show`: bare short ID, then canon's q/a labels and escaped values.
    format!(
        "{}\nq: {}\na: {}\n",
        expectation.display_id,
        escape_check_output_text(&expectation.question),
        escape_check_output_text(expectation.expected_answer())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 2gZ
    fn show_record_has_bare_id_and_lowercase_expected_label() {
        let expectation = ResolvedExpectation {
            kind: crate::check::core::ResolvedExpectationKind::Configured {
                id: "11111111111111111111".to_string(),
            },
            display_id: "1".to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            question: "Line one\nLine two".to_string(),
            expected_answer: "yes\tplease".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: Default::default(),
            cooldown: None,
            q_scope: Default::default(),
        };

        assert_eq!(
            render_canonical_show_record(&expectation),
            "1\nq: Line one\\nLine two\na: yes\\tplease\n"
        );
    }
}
