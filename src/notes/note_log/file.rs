use crate::fs_util::reject_symlink;
use crate::project_types::Note;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

pub(super) fn open_note_reader(note: &Note) -> Result<BufReader<fs::File>, String> {
    reject_symlink(&note.path)?;
    let file = fs::File::open(&note.path).map_err(|err| note_read_error(note, err))?;
    Ok(BufReader::new(file))
}

pub(super) fn read_note_storage_line(
    note: &Note,
    reader: &mut impl BufRead,
    offset: &mut usize,
) -> Result<Option<(usize, String)>, String> {
    let mut line = String::new();
    let line_start = *offset;
    let read = reader
        .read_line(&mut line)
        .map_err(|err| note_read_error(note, err))?;
    if read == 0 {
        return Ok(None);
    }
    *offset += read;
    Ok(Some((line_start, line)))
}

pub(crate) fn read_note_data<T>(
    note: &Note,
    read: impl FnOnce(&Path) -> io::Result<T>,
) -> Result<T, String> {
    reject_symlink(&note.path)?;
    read(&note.path).map_err(|err| note_read_error(note, err))
}

fn note_read_error(note: &Note, err: io::Error) -> String {
    format!("failed to read {}: {}", note.path.display(), err)
}
