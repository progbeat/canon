use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const ZERO_TOKEN_USAGE_LINE: &str =
    "token-usage: ref-cost=0.00$ total=0 input=0 (+ 0 cached) output=0 (reasoning 0)\n";
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn canon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_canon"))
}

fn portable_temp_dir(prefix: &str) -> PathBuf {
    // [Pi] These integration tests use only portable standard-library path,
    // filesystem, process, and temporary-directory APIs. Assertions describe
    // public CLI behavior and never depend on host path syntax or OS details.
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("{prefix}-{unique}-{sequence}"))
}

fn committed_git_project(prefix: &str) -> Result<PathBuf, String> {
    // [Pi] Git-backed CLI tests use Git's cross-platform command interface as
    // fixture setup. No test branches on an operating system or Git's on-disk
    // implementation, and every project gets an isolated committed history.
    let project = portable_temp_dir(prefix);
    fs::create_dir_all(&project).map_err(|err| err.to_string())?;
    for args in [
        &["init", "--quiet"][..],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "--quiet",
            "-m",
            "initial",
        ][..],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&project)
            .output()
            .map_err(|err| format!("failed to run git fixture setup: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "git fixture setup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    Ok(project)
}

#[path = "examples/caller.rs"]
mod caller;
#[path = "examples/gate.rs"]
mod gate;
#[path = "examples/in_place.rs"]
mod in_place;
#[path = "examples/initialization.rs"]
mod initialization;
#[path = "examples/state_write.rs"]
mod state_write;
#[path = "examples/validation.rs"]
mod validation;
