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
