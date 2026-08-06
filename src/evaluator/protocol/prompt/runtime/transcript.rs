#[derive(Clone)]
pub(crate) struct ShTranscriptMarkers {
    pub(crate) start: String,
    pub(crate) end: String,
    pub(crate) escape: String,
}

impl ShTranscriptMarkers {
    pub(crate) fn new() -> Result<ShTranscriptMarkers, String> {
        let nonce = getrandom::u64()
            .map_err(|err| format!("failed to choose prompt template sentinel: {err}"))?;
        Ok(ShTranscriptMarkers {
            start: format!("\x1Fcanon-sh-transcript-start-{nonce:016x}\x1F"),
            end: format!("\x1Fcanon-sh-transcript-end-{nonce:016x}\x1F"),
            escape: format!("\x1Fcanon-sh-transcript-escape-{nonce:016x}\x1F"),
        })
    }

    pub(crate) fn wrap_transcript(&self, transcript: &str) -> String {
        format!(
            "{}{}{}",
            self.start,
            encode_sh_transcript_marker_text(transcript, self),
            self.end
        )
    }
}

pub(crate) fn trim_rendered_prompt_template_output(
    rendered: &str,
    sh_transcript_markers: &ShTranscriptMarkers,
) -> String {
    let mut output = String::new();
    let mut rest = rendered.trim();
    while let Some(start_index) = rest.find(&sh_transcript_markers.start) {
        output.push_str(&rest[..start_index]);
        let after_start = &rest[start_index + sh_transcript_markers.start.len()..];
        let Some(end_index) = after_start.find(&sh_transcript_markers.end) else {
            output.push_str(&rest[start_index..]);
            return output;
        };
        output.push_str(&decode_sh_transcript_marker_text(
            &after_start[..end_index],
            sh_transcript_markers,
        ));
        rest = &after_start[end_index + sh_transcript_markers.end.len()..];
    }
    output.push_str(rest);
    output
}

fn encode_sh_transcript_marker_text(transcript: &str, markers: &ShTranscriptMarkers) -> String {
    transcript
        .replace(&markers.escape, &(markers.escape.clone() + "e"))
        .replace(&markers.start, &(markers.escape.clone() + "s"))
        .replace(&markers.end, &(markers.escape.clone() + "n"))
}

fn decode_sh_transcript_marker_text(encoded: &str, markers: &ShTranscriptMarkers) -> String {
    let mut output = String::new();
    let mut rest = encoded;
    while let Some(index) = rest.find(&markers.escape) {
        output.push_str(&rest[..index]);
        rest = &rest[index + markers.escape.len()..];
        let Some(code) = rest.chars().next() else {
            output.push_str(&markers.escape);
            return output;
        };
        match code {
            'e' => output.push_str(&markers.escape),
            's' => output.push_str(&markers.start),
            'n' => output.push_str(&markers.end),
            _ => {
                output.push_str(&markers.escape);
                output.push(code);
            }
        }
        rest = &rest[code.len_utf8()..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 3a
    fn outer_trim_preserves_shell_transcript_edges() {
        let markers = test_markers();
        let transcript = "$ cmd\n  output  \n";
        let rendered = format!("\n  {}  \n", markers.wrap_transcript(transcript));

        assert_eq!(
            trim_rendered_prompt_template_output(&rendered, &markers),
            transcript
        );
    }

    #[test] // xpec: 3a
    fn shell_transcript_markers_are_preserved_inside_transcript_text() {
        let markers = test_markers();
        let transcript = format!(
            "$ cmd\n{}{}{}\n",
            markers.start, markers.end, markers.escape
        );
        let rendered = markers.wrap_transcript(&transcript);

        assert_eq!(
            trim_rendered_prompt_template_output(&rendered, &markers),
            transcript
        );
    }

    fn test_markers() -> ShTranscriptMarkers {
        ShTranscriptMarkers {
            start: "<start>".to_string(),
            end: "<end>".to_string(),
            escape: "<escape>".to_string(),
        }
    }
}
