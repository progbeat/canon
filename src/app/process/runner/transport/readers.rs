use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

pub(crate) fn spawn_app_server_reader(
    stdout: std::process::ChildStdout,
) -> (Receiver<Result<Value, String>>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {
                    let parsed = serde_json::from_str(line.trim_end())
                        .map_err(|err| format!("failed to parse app-server JSON: {}", err));
                    if sender.send(parsed).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ =
                        sender.send(Err(format!("failed to read app-server response: {}", err)));
                    return;
                }
            }
        }
    });
    (receiver, reader)
}

pub(crate) fn spawn_app_server_stderr_reader(
    stderr: std::process::ChildStderr,
) -> (Receiver<Vec<u8>>, JoinHandle<()>) {
    spawn_app_server_stderr_reader_with_forwarder(stderr, |_| Ok(()))
}

pub(crate) fn spawn_app_server_stderr_reader_with_forwarder<R, F>(
    mut stderr: R,
    forward: F,
) -> (Receiver<Vec<u8>>, JoinHandle<()>)
where
    R: Read + Send + 'static,
    F: Fn(&[u8]) -> Result<(), String> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => return,
                Ok(n) => {
                    let bytes = &buffer[..n];
                    let _ = forward(bytes);
                    let _ = sender.send(bytes.to_vec());
                }
                Err(_) => return,
            }
        }
    });
    (receiver, reader)
}

#[cfg(test)]
mod tests {
    use super::spawn_app_server_stderr_reader_with_forwarder;
    use std::collections::VecDeque;
    use std::io::{self, Read};
    use std::sync::mpsc;
    use std::time::Duration;

    struct ChannelReader {
        chunks: mpsc::Receiver<Vec<u8>>,
        pending: VecDeque<u8>,
    }

    impl ChannelReader {
        fn new(chunks: mpsc::Receiver<Vec<u8>>) -> Self {
            Self {
                chunks,
                pending: VecDeque::new(),
            }
        }
    }

    impl Read for ChannelReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            while self.pending.is_empty() {
                match self.chunks.recv() {
                    Ok(chunk) => self.pending.extend(chunk),
                    Err(_) => return Ok(0),
                }
            }

            let length = buffer.len().min(self.pending.len());
            for destination in &mut buffer[..length] {
                if let Some(byte) = self.pending.pop_front() {
                    *destination = byte;
                }
            }
            Ok(length)
        }
    }

    #[test] // xpec: 1h
    fn stderr_reader_forwards_before_eof() {
        let (input_tx, input_rx) = mpsc::channel();
        let (forwarded_tx, forwarded_rx) = mpsc::channel();
        let (captured_rx, handle) = spawn_app_server_stderr_reader_with_forwarder(
            ChannelReader::new(input_rx),
            move |bytes| {
                forwarded_tx
                    .send(bytes.to_vec())
                    .map_err(|err| err.to_string())
            },
        );

        input_tx.send(b"early stderr\n".to_vec()).unwrap();

        assert_eq!(
            forwarded_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"early stderr\n"
        );
        assert_eq!(
            captured_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"early stderr\n"
        );

        drop(input_tx);
        handle.join().unwrap();
    }
}
