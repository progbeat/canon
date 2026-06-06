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
        writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))?;
        writer.flush()
    }
}

pub(crate) fn write_stdout_line_record(
    writer: &mut dyn Write,
    line: &str,
    description: &str,
) -> Result<(), String> {
    let mut output = String::with_capacity(line.len() + 1);
    output.push_str(line);
    output.push('\n');
    write_stdout_record(writer, output.as_bytes(), description)
}

pub(super) fn write_stdout_record(
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
