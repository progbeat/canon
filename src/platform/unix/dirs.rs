use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    private_dir_builder(false).create(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    private_dir_builder(true).create(path)
}

fn private_dir_builder(recursive: bool) -> fs::DirBuilder {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    builder.mode(0o700);
    builder
}
