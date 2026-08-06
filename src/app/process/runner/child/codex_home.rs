use crate::evaluator::EvaluatorProcessIsolation;
use crate::evaluator_sandbox_filesystem::allocate_evaluator_runtime_directory;
use crate::fs_util::ensure_dir_without_symlinks;
use crate::platform::filesystem::{
    self, OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// Codex exposes some startup inputs only through files under CODEX_HOME.
// This owner keeps those inputs in private temporary directories; Canon never
// reads them back as evaluator state.
const EVALUATOR_CODEX_HOME_AUTH_FILES: &[&str] = &["auth.json", "installation_id", "version.json"];
const EVALUATOR_CODEX_HOME_PREFIX: &str = "canon-evaluator-codex-home";
const SYSTEM_SKILLS_INPUT_MARKER: &str = ".codex-system-skills.marker";

pub(crate) struct EvaluatorCodexHome {
    ephemeral_runtime_root: OwnedPrivateTemporaryDirectory,
    path: PathBuf,
}

impl EvaluatorCodexHome {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn runtime_root(&self) -> &Path {
        self.ephemeral_runtime_root.path()
    }

    pub(crate) fn materialize_runtime_executable(&self, source: &Path) -> Result<PathBuf, String> {
        let file_name = source.file_name().ok_or_else(|| {
            format!(
                "failed to identify evaluator executable file name for {}",
                source.display()
            )
        })?;
        let target = self.runtime_root().join(file_name);
        match fs::hard_link(source, &target) {
            Ok(()) => Ok(target),
            Err(hard_link_error) => match fs::copy(source, &target) {
                Ok(_) => Ok(target),
                Err(copy_error) => Err(format!(
                    "failed to materialize evaluator executable {} from {}: \
                         hardlink failed: {}; copy failed: {}",
                    target.display(),
                    source.display(),
                    hard_link_error,
                    copy_error
                )),
            },
        }
    }
}

pub(crate) fn prepare_evaluator_codex_home(
    process_isolation: EvaluatorProcessIsolation,
) -> Result<EvaluatorCodexHome, String> {
    let codex_home = allocate_evaluator_codex_home(process_isolation)?;
    // [g2] This directory layout is the filesystem interface required by the
    // external Codex process, not Canon invocation state. Canon never reads it
    // back to make check decisions, and its lifetime owner removes the whole
    // private root when the app-server exits.
    ensure_evaluator_codex_home_dir(codex_home.path())?;
    for dir in [
        ".tmp", "cache", "log", "mcp", "memories", "plugins", "sessions", "skills",
    ] {
        ensure_evaluator_codex_home_dir(&codex_home.path().join(dir))?;
    }
    let source_home = source_codex_home();
    materialize_system_skills_input(source_home.as_deref(), codex_home.path())?;
    if let Some(source_home) = source_home {
        if !same_existing_path(&source_home, codex_home.path()) {
            for file_name in EVALUATOR_CODEX_HOME_AUTH_FILES {
                mirror_codex_home_file(&source_home, codex_home.path(), file_name)?;
            }
        }
    }
    Ok(codex_home)
}

fn allocate_evaluator_codex_home(
    process_isolation: EvaluatorProcessIsolation,
) -> Result<EvaluatorCodexHome, String> {
    let allocator = PrivateTemporaryDirectoryAllocator::new();
    // [KD,Mo] Select storage that satisfies each execution mode's filesystem
    // contract while retaining one lifetime owner for cleanup.
    // Canon-managed Codex creates executable arg0 helpers below CODEX_HOME, so
    // its runtime root needs the same executable and sandbox-visibility
    // guarantees as the materialized app-server binary. An externally managed
    // evaluator runs the installed binary and uses the caller's portable temp
    // storage contract.
    let ephemeral_runtime_root = match process_isolation {
        EvaluatorProcessIsolation::CanonManaged => {
            allocate_evaluator_runtime_directory(&allocator, EVALUATOR_CODEX_HOME_PREFIX)?
        }
        EvaluatorProcessIsolation::ExternallyManaged => {
            OwnedPrivateTemporaryDirectory::create(&allocator, EVALUATOR_CODEX_HOME_PREFIX)?
        }
    };
    let path = ephemeral_runtime_root.path().join(".codex");
    Ok(EvaluatorCodexHome {
        ephemeral_runtime_root,
        path,
    })
}

fn ensure_evaluator_codex_home_dir(path: &Path) -> Result<(), String> {
    ensure_dir_without_symlinks(path)
}

fn materialize_system_skills_input(
    source_home: Option<&Path>,
    target_home: &Path,
) -> Result<(), String> {
    let system_dir = target_home.join("skills").join(".system");
    ensure_evaluator_codex_home_dir(&system_dir)?;
    let target = system_dir.join(SYSTEM_SKILLS_INPUT_MARKER);
    if let Some(source) = source_home.map(|source_home| {
        source_home
            .join("skills")
            .join(".system")
            .join(SYSTEM_SKILLS_INPUT_MARKER)
    }) {
        if source.is_file() {
            fs::copy(&source, &target).map_err(|err| {
                format!(
                    "failed to copy evaluator system skills marker {} from {}: {}",
                    target.display(),
                    source.display(),
                    err
                )
            })?;
            return Ok(());
        }
    }
    // [g2] The marker is an immutable Codex startup input that suppresses
    // implicit system-skill discovery. It carries no Canon execution state.
    fs::write(&target, b"canon-empty-system-skills\n")
        .map_err(|err| format!("failed to write {}: {}", target.display(), err))
}

fn source_codex_home() -> Option<PathBuf> {
    env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

fn mirror_codex_home_file(
    source_home: &Path,
    target_home: &Path,
    file_name: &str,
) -> Result<(), String> {
    let source = source_home.join(file_name);
    if !source.is_file() {
        return Ok(());
    }
    let target = target_home.join(file_name);
    if same_existing_path(&source, &target) {
        return Ok(());
    }
    remove_existing_codex_home_entry(&target)?;
    filesystem::mirror_evaluator_codex_home_file(&source, &target)
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn remove_existing_codex_home_entry(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|err| format!("failed to replace {}: {}", path.display(), err))
}
