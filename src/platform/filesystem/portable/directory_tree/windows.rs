use super::super::permissions::windows::set_private_permissions_with_metadata;
use super::{DirectoryRegistrar, DirectoryTreeRegistration};
use std::fs;
use std::io;
use std::path::Path;

struct PrivateDirectoryRegistrar;

impl DirectoryRegistrar for PrivateDirectoryRegistrar {
    type Error = String;

    fn register(&mut self, path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
        set_private_permissions_with_metadata(path, metadata)
    }

    fn filesystem_error(&self, context: String, source: io::Error) -> String {
        format!("{context}: {source}")
    }
}

pub(in super::super) fn make_directory_tree_private(path: &Path) -> Result<(), String> {
    DirectoryTreeRegistration::new(PrivateDirectoryRegistrar).extend(path)
}
