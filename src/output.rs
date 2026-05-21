use std::io::{self, Write};

pub(crate) fn write_stdout(text: &str) -> Result<(), String> {
    write_stdout_bytes(text.as_bytes())
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> Result<(), String> {
    let stdout = io::stdout();
    write_and_flush(stdout.lock(), "stdout", bytes)
}

pub(crate) fn write_stdout_line(text: &str) -> Result<(), String> {
    let stdout = io::stdout();
    write_line_and_flush(stdout.lock(), "stdout", text)
}

pub(crate) fn write_stderr(text: &str) -> Result<(), String> {
    write_stderr_bytes(text.as_bytes())
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
