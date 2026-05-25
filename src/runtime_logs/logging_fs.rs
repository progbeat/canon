use crate::logging_error::{log_io_error, DiagnosticLogResult};
use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn remove_file_if_exists(path: &Path) -> DiagnosticLogResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(log_io_error("remove", path, err)),
    }
}
