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
) -> (Receiver<String>, JoinHandle<()>) {
    spawn_app_server_stderr_reader_with_forwarder(stderr, |_| Ok(()))
}

pub(crate) fn spawn_app_server_stderr_reader_with_forwarder<R, F>(
    mut stderr: R,
    forward: F,
) -> (Receiver<String>, JoinHandle<()>)
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
                    let _ = sender.send(String::from_utf8_lossy(bytes).into_owned());
                }
                Err(_) => return,
            }
        }
    });
    (receiver, reader)
}

#[cfg(all(test, unix))]
mod tests {
    use super::spawn_app_server_stderr_reader_with_forwarder;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test] // xpec: 1h
    fn stderr_reader_forwards_before_eof() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let (forwarded_tx, forwarded_rx) = mpsc::channel();
        let (captured_rx, handle) =
            spawn_app_server_stderr_reader_with_forwarder(reader, move |bytes| {
                forwarded_tx
                    .send(bytes.to_vec())
                    .map_err(|err| err.to_string())
            });

        writer.write_all(b"early stderr\n").unwrap();

        assert_eq!(
            forwarded_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"early stderr\n"
        );
        assert_eq!(
            captured_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "early stderr\n"
        );

        drop(writer);
        handle.join().unwrap();
    }
}
