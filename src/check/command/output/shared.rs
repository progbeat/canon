use std::io::{self, Write};
use std::sync::{Arc, Mutex};

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
}

impl Write for SharedCheckOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))?;
        let written = writer.write(bytes)?;
        // Progress output is intentionally visible at fragment boundaries,
        // including partial records such as short IDs and timeline markers.
        if written > 0 {
            writer.flush()?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))?;
        writer.flush()
    }
}

pub(crate) fn write_stdout_record(
    writer: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> Result<(), String> {
    writer
        .write_all(bytes)
        .map_err(|err| format!("failed to write {} to stdout: {}", description, err))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush {} to stdout: {}", description, err))
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

    // xpec: j
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
