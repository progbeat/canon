use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

mod cache;
mod in_place;
mod staged_content;

fn test_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp")
        .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn outside_test_file(root: &Path) -> PathBuf {
    let file_name = root
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("canon-test");
    root.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{file_name}-outside"))
}
