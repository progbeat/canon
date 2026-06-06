use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) root: PathBuf,
}

#[derive(Debug)]
pub(crate) struct Note {
    pub(crate) key: String,
    pub(crate) hash: String,
    pub(crate) path: PathBuf,
}
