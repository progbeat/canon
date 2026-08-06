use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub(crate) struct SharedCheckOutput {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl SharedCheckOutput {
    pub(crate) fn stdout() -> SharedCheckOutput {
        SharedCheckOutput::new(Box::new(io::stdout()))
    }

    pub(crate) fn new(writer: Box<dyn Write + Send>) -> SharedCheckOutput {
        SharedCheckOutput {
            inner: Arc::new(Mutex::new(writer)),
        }
    }

    fn lock_writer(&self) -> io::Result<MutexGuard<'_, Box<dyn Write + Send>>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))
    }

    pub(crate) fn write_fragment(
        &self,
        bytes: &[u8],
        description: &str,
    ) -> StdoutFragmentWriteOutcome {
        match self.lock_writer() {
            Ok(mut writer) => write_stdout_fragment_with_outcome(&mut **writer, bytes, description),
            Err(err) => StdoutFragmentWriteOutcome::failed(format!(
                "failed to write {} to stdout: {}",
                description, err
            )),
        }
    }
}

impl Write for SharedCheckOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut writer = self.lock_writer()?;
        let written = writer.write(bytes)?;
        // Progress output is intentionally visible at fragment boundaries,
        // including short IDs and timeline markers within a live report.
        if written > 0 {
            writer.flush()?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut writer = self.lock_writer()?;
        writer.flush()
    }
}

pub(crate) fn write_stdout_record(
    writer: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> Result<(), String> {
    write_stdout_fragment_with_outcome(writer, bytes, description).into_result()
}

pub(crate) struct StdoutFragmentWriteOutcome {
    entire_fragment_was_written: bool,
    result: Result<(), String>,
}

impl StdoutFragmentWriteOutcome {
    fn failed(error: String) -> StdoutFragmentWriteOutcome {
        StdoutFragmentWriteOutcome {
            entire_fragment_was_written: false,
            result: Err(error),
        }
    }

    pub(crate) fn entire_fragment_was_written(&self) -> bool {
        self.entire_fragment_was_written
    }

    pub(crate) fn completed(&self) -> bool {
        self.result.is_ok()
    }

    pub(crate) fn into_result(self) -> Result<(), String> {
        self.result
    }
}

fn write_stdout_fragment_with_outcome(
    writer: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> StdoutFragmentWriteOutcome {
    // [sy] Under `Write`'s public contract, accepted bytes are reported only by
    // `Ok(n)`, and `write_all` succeeds only after all requested bytes have
    // been accepted. An `Err` may follow partial output, but side effects that a
    // non-conforming writer performs while reporting `Err` are unknowable to
    // its caller and do not turn that result into successful fragment
    // acceptance. Keep complete acceptance separate from the following flush
    // result so a complete short ID remains recognizable as the public report
    // even when that flush fails.
    if let Err(err) = writer.write_all(bytes) {
        return StdoutFragmentWriteOutcome::failed(format!(
            "failed to write {} to stdout: {}",
            description, err
        ));
    }
    let result = writer
        .flush()
        .map_err(|err| format!("failed to flush {} to stdout: {}", description, err));
    StdoutFragmentWriteOutcome {
        entire_fragment_was_written: true,
        result,
    }
}

pub(crate) fn write_stdout_message_lines(
    writer: &mut dyn Write,
    messages: impl IntoIterator<Item = String>,
    description: &str,
) -> Result<(), String> {
    for mut message in messages {
        message.push('\n');
        write_stdout_record(writer, message.as_bytes(), description)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{write_stdout_record, SharedCheckOutput};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq)]
    enum Event {
        Write(Vec<u8>),
        Flush,
    }

    #[derive(Clone)]
    struct RecordingWriter {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.events
                .lock()
                .unwrap()
                .push(Event::Write(bytes.to_vec()));
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.events.lock().unwrap().push(Event::Flush);
            Ok(())
        }
    }

    // xpec: 1h
    #[test]
    fn check_output_helpers_flush_eligible_stdout_fragments() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut writer = RecordingWriter {
            events: events.clone(),
        };

        write_stdout_record(&mut writer, b"record", "test record").unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec![Event::Write(b"record".to_vec()), Event::Flush]
        );

        let events = Arc::new(Mutex::new(Vec::new()));
        let mut output = SharedCheckOutput::new(Box::new(RecordingWriter {
            events: events.clone(),
        }));

        output.write_all(b".").unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec![Event::Write(b".".to_vec()), Event::Flush]
        );
    }
}
