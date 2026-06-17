use crate::fs_util::ensure_dir_without_symlinks;
use crate::hash::hash_60;
use crate::platform;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const EVALUATOR_CODEX_HOME_AUTH_FILES: &[&str] = &["auth.json", "installation_id", "version.json"];
const SYSTEM_SKILLS_MARKER: &str = ".codex-system-skills.marker";
const EVALUATOR_CODEX_HOME_RESET_DIRS: &[&str] =
    &["mcp", "memories", "plugins", "sessions", "skills"];
const EVALUATOR_CODEX_HOME_RESET_FILES: &[&str] = &[
    "AGENTS.md",
    "config.json",
    "config.toml",
    "instructions.md",
    "preferences.json",
];

pub(crate) fn prepare_evaluator_codex_home(root: &Path) -> Result<PathBuf, String> {
    let codex_home = evaluator_codex_home_path(root)?;
    ensure_evaluator_codex_home_dir(&codex_home)?;
    for file in EVALUATOR_CODEX_HOME_RESET_FILES {
        remove_existing_codex_home_entry(&codex_home.join(file))?;
    }
    for dir in EVALUATOR_CODEX_HOME_RESET_DIRS {
        remove_existing_codex_home_entry(&codex_home.join(dir))?;
    }
    for dir in [
        ".tmp", "cache", "log", "mcp", "memories", "plugins", "sessions", "skills",
    ] {
        ensure_evaluator_codex_home_dir(&codex_home.join(dir))?;
    }
    let source_home = source_codex_home();
    write_empty_system_skills_marker(source_home.as_deref(), &codex_home)?;
    if let Some(source_home) = source_home {
        if !same_existing_path(&source_home, &codex_home) {
            for file_name in EVALUATOR_CODEX_HOME_AUTH_FILES {
                mirror_codex_home_file(&source_home, &codex_home, file_name)?;
            }
        }
    }
    Ok(codex_home)
}

fn evaluator_codex_home_path(root: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize evaluator root: {}", err))?;
    let temp_root = env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))?;
    let root_key = hash_60(root.to_string_lossy().as_bytes());
    Ok(temp_root
        .join("canon-evaluator-codex-home")
        .join(root_key)
        .join(".codex"))
}

fn ensure_evaluator_codex_home_dir(path: &Path) -> Result<(), String> {
    ensure_dir_without_symlinks(path)
}

fn write_empty_system_skills_marker(
    source_home: Option<&Path>,
    target_home: &Path,
) -> Result<(), String> {
    let system_dir = target_home.join("skills").join(".system");
    ensure_evaluator_codex_home_dir(&system_dir)?;
    let target = system_dir.join(SYSTEM_SKILLS_MARKER);
    if let Some(source) = source_home.map(|source_home| {
        source_home
            .join("skills")
            .join(".system")
            .join(SYSTEM_SKILLS_MARKER)
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
    platform::mirror_evaluator_codex_home_file(&source, &target)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluator_codex_home_path_does_not_encode_repo_root() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap();
        let codex_home = evaluator_codex_home_path(&root).unwrap();

        assert_eq!(codex_home.file_name(), Some(std::ffi::OsStr::new(".codex")));
        assert!(codex_home.starts_with(env::temp_dir().canonicalize().unwrap()));
        assert!(!codex_home.starts_with(&root));
        assert!(!codex_home.to_string_lossy().contains(&root.to_string_lossy()[..]));
    }
}
