use super::*;
use std::fs;

#[test] // xpec: d
fn stdout_artifact_cache_materializes_each_complete_content_once() {
    use std::cell::Cell;

    let cache = PromptTemplateArtifactDirCache::new(PrivateTemporaryDirectoryAllocator::new());
    let materialization_calls = Cell::new(0);
    let first = cache
        .materialize_stdout_artifact(
            b"first",
            |dir| dir.join("first"),
            |path| {
                materialization_calls.set(materialization_calls.get() + 1);
                fs::write(path, b"first").map_err(|err| err.to_string())
            },
        )
        .unwrap();
    let first_again = cache
        .materialize_stdout_artifact(
            b"first",
            |_| PathBuf::from("must-not-be-used"),
            |_| panic!("cached stdout must not be materialized again"),
        )
        .unwrap();
    let second = cache
        .materialize_stdout_artifact(
            b"second",
            |dir| dir.join("second"),
            |path| {
                materialization_calls.set(materialization_calls.get() + 1);
                fs::write(path, b"second").map_err(|err| err.to_string())
            },
        )
        .unwrap();

    assert_eq!(first, first_again);
    assert_ne!(first, second);
    assert_eq!(fs::read(first).unwrap(), b"first");
    assert_eq!(fs::read(second).unwrap(), b"second");
    assert_eq!(materialization_calls.get(), 2);
}
