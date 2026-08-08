use super::*;
use crate::config_types::AgentConfig;
use std::fs;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: 90
fn in_place_runner_initializes_outside_git_worktree() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "canon-in-place-runner-no-git-{}-{}",
        process::id(),
        stamp
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    LazyAppServerRunner::new_in_place(
        false,
        &AgentConfig::default(),
        EvaluatorProcessIsolation::CanonManaged,
    )
    .unwrap();
    fs::remove_dir_all(root).unwrap();
}
