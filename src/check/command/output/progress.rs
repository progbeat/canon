use super::record::render_check_output_record_completion;
use super::shared::{write_stdout_record, SharedCheckOutput};
use crate::check::core::types::CheckRecord;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PROGRESS_DOT_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) struct CheckProgressOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

pub(crate) fn start_check_progress_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> Result<CheckProgressOutput, String> {
    let mut immediate_output = output.clone();
    write_stdout_record(
        &mut immediate_output,
        format!("{}.", display_id).as_bytes(),
        "check progress prefix",
    )?;

    let (stop, stop_requested) = mpsc::channel();
    let mut progress_output = output.clone();
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(PROGRESS_DOT_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                write_stdout_record(&mut progress_output, b".", "check progress dot")?;
            }
        }
    });

    Ok(CheckProgressOutput {
        output,
        stop,
        worker: Some(worker),
    })
}

impl CheckProgressOutput {
    pub(crate) fn finish_with_record(mut self, record: &CheckRecord) -> Result<(), String> {
        self.stop_progress_worker()?;
        let completion = render_check_output_record_completion(record);
        let mut output = self.output.clone();
        write_stdout_record(&mut output, completion.as_bytes(), "check result")
    }

    pub(crate) fn cancel(mut self) -> Result<(), String> {
        self.stop_progress_worker()
    }

    fn stop_progress_worker(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check progress thread panicked".to_string())?
    }
}
