use std::io::{self, Write};

pub(crate) fn command_output_trimmed<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    Ok(std::str::from_utf8(bytes)
        .map_err(|err| format!("{} must be valid UTF-8: {}", description, err))?
        .trim())
}

// General command output goes through these helpers so each eligible stdout or
// stderr fragment is flushed before the caller can continue. Commands with
// streaming output, such as `canon check`, own a command-specific facade with
// the same flush-after-write contract. A write or flush error means the host
// did not accept that public-output fragment; callers can report or ignore
// that I/O failure, but cannot turn a broken output sink into printed bytes.
pub(crate) fn write_stdout(text: &str) -> Result<(), String> {
    write_stdout_bytes(text.as_bytes())
}

pub(crate) fn write_stderr(text: &str) -> Result<(), String> {
    write_stderr_bytes(text.as_bytes())
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> Result<(), String> {
    let stdout = io::stdout();
    write_and_flush(stdout.lock(), "stdout", bytes)
}

pub(crate) fn write_stdout_line(text: &str) -> Result<(), String> {
    let stdout = io::stdout();
    write_line_and_flush(stdout.lock(), "stdout", text)
}

pub(crate) fn write_stderr_bytes(bytes: &[u8]) -> Result<(), String> {
    let stderr = io::stderr();
    write_and_flush(stderr.lock(), "stderr", bytes)
}

pub(crate) fn write_stderr_line(text: &str) -> Result<(), String> {
    let stderr = io::stderr();
    write_line_and_flush(stderr.lock(), "stderr", text)
}

fn write_and_flush(mut writer: impl Write, stream: &str, bytes: &[u8]) -> Result<(), String> {
    write_segments_and_flush(&mut writer, stream, &[bytes])
}

fn write_line_and_flush(mut writer: impl Write, stream: &str, text: &str) -> Result<(), String> {
    write_segments_and_flush(&mut writer, stream, &[text.as_bytes(), b"\n"])
}

fn write_segments_and_flush(
    writer: &mut impl Write,
    stream: &str,
    segments: &[&[u8]],
) -> Result<(), String> {
    for bytes in segments {
        writer
            .write_all(bytes)
            .map_err(|err| format!("failed to write to {}: {}", stream, err))?;
    }
    writer
        .flush()
        .map_err(|err| format!("failed to flush {}: {}", stream, err))
}

#[cfg(test)]
mod tests {
    use super::write_segments_and_flush;
    use std::io::{self, Write};

    #[derive(Debug, PartialEq)]
    enum Event {
        Write(Vec<u8>),
        Flush,
    }

    struct RecordingWriter {
        events: Vec<Event>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.events.push(Event::Write(bytes.to_vec()));
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.events.push(Event::Flush);
            Ok(())
        }
    }

    // xpec: 1h
    #[test]
    fn general_output_helpers_flush_eligible_stream_fragments() {
        let mut writer = RecordingWriter { events: Vec::new() };

        write_segments_and_flush(&mut writer, "stdout", &[b"hello", b"\n"]).unwrap();

        assert_eq!(
            writer.events,
            vec![
                Event::Write(b"hello".to_vec()),
                Event::Write(b"\n".to_vec()),
                Event::Flush,
            ]
        );
    }
}
