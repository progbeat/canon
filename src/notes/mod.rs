mod cli;
mod header;
mod index;
mod note_lock;
mod note_log;
mod restore;
mod store;

pub(crate) use cli::{arg_to_string, collect_text_or_stdin, require_key, run_rg};
pub(crate) use store::{append_note, delete_note, ensure_note, read_note, write_note};
