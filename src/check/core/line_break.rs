use crate::logs::push_json_control_escape;

// Shared by config validation and check-output escaping for text that must
// stay visually on one terminal/config line.
pub(crate) fn is_line_break_char(char: char) -> bool {
    matches!(
        char,
        '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

pub(crate) fn contains_line_break(value: &str) -> bool {
    value.chars().any(is_line_break_char)
}

pub(crate) fn escape_inline_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if is_line_break_char(ch) || ch.is_control() => {
                push_unicode_escape(&mut output, ch);
            }
            ch => output.push(ch),
        }
    }
    output
}

fn push_unicode_escape(output: &mut String, ch: char) {
    if (ch as u32) <= 0xff {
        push_json_control_escape(output, ch as u8);
    } else {
        let mut units = [0; 2];
        for unit in ch.encode_utf16(&mut units) {
            output.push_str(&format!("\\u{unit:04x}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::escape_inline_text;

    #[test] // xpec: gN
    fn inline_escape_distinguishes_line_breaks_from_literal_escape_text() {
        assert_eq!(escape_inline_text("\n"), "\\n");
        assert_eq!(escape_inline_text("\\n"), "\\\\n");
    }
}
