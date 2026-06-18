use crate::fs_util::ensure_dir_without_symlinks;
use crate::platform;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const EVALUATOR_CODEX_HOME_AUTH_FILES: &[&str] = &["auth.json", "installation_id", "version.json"];
const EVALUATOR_CODEX_HOME_RANDOM_BYTES: usize = 16;
const EVALUATOR_CODEX_HOME_RANDOM_ATTEMPTS: usize = 1000;
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

fn evaluator_codex_home_path(_root: &Path) -> Result<PathBuf, String> {
    let temp_root = env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))?;
    for _ in 0..EVALUATOR_CODEX_HOME_RANDOM_ATTEMPTS {
        let parent = temp_root.join(format!(
            "canon-evaluator-codex-home-{}",
            random_hex(EVALUATOR_CODEX_HOME_RANDOM_BYTES)?
        ));
        match platform::create_private_dir(&parent) {
            Ok(()) => return Ok(parent.join(".codex")),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create {}: {}", parent.display(), err)),
        }
    }
    Err(format!(
        "failed to allocate unique evaluator Codex home under {}",
        temp_root.display()
    ))
}

fn ensure_evaluator_codex_home_dir(path: &Path) -> Result<(), String> {
    ensure_dir_without_symlinks(path)
}

fn random_hex(len: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes)
        .map_err(|err| format!("failed to generate evaluator Codex home name: {}", err))?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
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
    fn evaluator_codex_home_path_uses_private_random_temp_parent() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let first = evaluator_codex_home_path(&root).unwrap();
        let second = evaluator_codex_home_path(&root).unwrap();

        assert_ne!(first, second);
        for codex_home in [&first, &second] {
            assert_eq!(codex_home.file_name(), Some(std::ffi::OsStr::new(".codex")));
            assert!(codex_home.starts_with(env::temp_dir().canonicalize().unwrap()));
            assert!(!codex_home.starts_with(&root));
            assert!(!codex_home
                .to_string_lossy()
                .contains(&root.to_string_lossy()[..]));
            assert!(codex_home.parent().unwrap().is_dir());
        }
        let _ = fs::remove_dir_all(first.parent().unwrap());
        let _ = fs::remove_dir_all(second.parent().unwrap());
    }
}
