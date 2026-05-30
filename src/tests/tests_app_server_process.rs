use super::*;

#[cfg(unix)]
#[test]
fn app_server_stderr_reader_forwards_before_eof() {
    use crate::app::process::spawn_app_server_stderr_reader_with_forwarder;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;

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
