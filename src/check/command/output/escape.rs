use crate::check::core::is_line_break_char;
use crate::logs::push_json_control_escape;

pub(crate) fn escape_check_output_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if is_line_break_char(ch) || ch.is_control() => {
                push_check_output_unicode_escape(&mut output, ch);
            }
            ch => output.push(ch),
        }
    }
    output
}

pub(crate) fn push_escaped_check_output_line(output: &mut String, label: &str, value: &str) {
    output.push_str(label);
    output.push_str(": ");
    output.push_str(&escape_check_output_text(value));
    output.push('\n');
}

fn push_check_output_unicode_escape(output: &mut String, ch: char) {
    if (ch as u32) <= 0xff {
        push_json_control_escape(output, ch as u8);
    } else {
        let mut units = [0; 2];
        for unit in ch.encode_utf16(&mut units) {
            output.push_str(&format!("\\u{unit:04x}"));
        }
    }
}
