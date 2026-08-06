use super::directory::PromptTemplateArtifactDir;
use crate::evaluator::protocol::prompt::runtime::template_error;
use crate::evaluator::protocol::prompt_artifact_permissions::{
    set_template_artifact_create_mode, set_template_artifact_file_permissions,
};
use minijinja::Error;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(super) fn write_full_template_command_stdout_artifact(
    template_artifact_dir: &PromptTemplateArtifactDir,
    stdout: &[u8],
) -> Result<PathBuf, Error> {
    match template_artifact_dir {
        PromptTemplateArtifactDir::Lazy(cache) => cache
            .materialize_stdout_artifact(
                stdout,
                |dir| template_command_stdout_artifact_path(dir, stdout),
                |path| materialize_template_command_stdout_artifact(path, stdout),
            )
            .map_err(template_error),
        #[cfg(test)]
        PromptTemplateArtifactDir::Fixed(dir) => {
            let path = template_command_stdout_artifact_path(dir, stdout);
            materialize_template_command_stdout_artifact(&path, stdout).map_err(template_error)?;
            Ok(path)
        }
    }
}

fn materialize_template_command_stdout_artifact(path: &Path, stdout: &[u8]) -> Result<(), String> {
    // This artifact is part of the evaluator-readable prompt transcript, not
    // canon check state. The implementation writes it so the truncation line's
    // path is readable by the evaluator, and never reads it back to make
    // check-run decisions.
    match create_template_command_stdout_artifact_file(path, stdout) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|read_err| {
                format!(
                    "failed to read existing prompt template output file {}: {}",
                    path.display(),
                    read_err
                )
            })?;
            if existing == stdout {
                return Ok(());
            }
            Err(format!(
                "prompt template output hash collision or stale file at {}",
                path.display()
            ))
        }
        Err(err) => Err(format!("failed to write {}: {}", path.display(), err)),
    }
}

pub(crate) fn template_command_stdout_artifact_path(
    template_output_dir: &Path,
    stdout: &[u8],
) -> PathBuf {
    // `template_output_dir` is stable for one check run, and the file name is
    // content-addressed by complete stdout bytes. Identical stdout therefore
    // maps to the same full path within that check run.
    template_output_dir.join(format!(
        "canon-template-output-sha256-{}.txt",
        sha256_hex(stdout)
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("hex digit nibble is always in range"),
    }
}

fn create_template_command_stdout_artifact_file(path: &Path, stdout: &[u8]) -> io::Result<()> {
    for _ in 0..16 {
        let temp_path = template_stdout_artifact_temp_path(path)?;
        match write_template_stdout_artifact_temp_file(&temp_path, stdout) {
            Ok(()) => {
                // Publish only after the temp sibling contains the complete
                // stdout. A concurrent process can then observe either no
                // content-addressed artifact or a complete one, never a
                // partially-written target file.
                let link_result = fs::hard_link(&temp_path, path);
                let _ = fs::remove_file(&temp_path);
                return link_result;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to choose a unique prompt template stdout artifact temp path",
    ))
}

fn template_stdout_artifact_temp_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "prompt template stdout artifact path has no parent",
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "prompt template stdout artifact path has no UTF-8 file name",
            )
        })?;
    let random = getrandom::u64().map_err(|err| {
        io::Error::other(format!("failed to choose stdout artifact temp path: {err}"))
    })?;
    Ok(parent.join(format!(".{file_name}.{}.{}", std::process::id(), random)))
}

fn write_template_stdout_artifact_temp_file(path: &Path, stdout: &[u8]) -> io::Result<()> {
    let mut file = template_artifact_open_options().open(path)?;
    let result = set_template_artifact_file_permissions(&file)
        .and_then(|()| file.write_all(stdout))
        .and_then(|()| file.flush());
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn template_artifact_open_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    set_template_artifact_create_mode(&mut options);
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::filesystem::create_private_dir;

    #[test] // xpec: 3a,d
    fn template_command_output_file_is_content_addressed_and_deduplicated() {
        let output = (0..1200)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("dedupe");
        let artifact_dir = PromptTemplateArtifactDir::Fixed(output_dir.clone());

        let first =
            write_full_template_command_stdout_artifact(&artifact_dir, output.as_bytes()).unwrap();
        let second =
            write_full_template_command_stdout_artifact(&artifact_dir, output.as_bytes()).unwrap();

        assert_eq!(first, second);
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("canon-template-output-sha256-"));
        assert_eq!(fs::read(&first).unwrap(), output.as_bytes());
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn template_command_stdout_path_is_deterministic_within_run_output_dir() {
        let output_dir = test_output_dir("same-run-content-addressed");
        let stdout = b"same complete stdout";
        let first = template_command_stdout_artifact_path(&output_dir, stdout);
        let second = template_command_stdout_artifact_path(&output_dir, stdout);

        assert_eq!(first, second);
        assert_eq!(first.parent(), Some(output_dir.as_path()));
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("canon-template-output-sha256-"));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn template_command_output_file_preserves_raw_stdout_bytes() {
        let mut output = (0..1200)
            .flat_map(|index| format!("line {index}\n").into_bytes())
            .collect::<Vec<_>>();
        output.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let output_dir = test_output_dir("raw-bytes");
        let artifact_dir = PromptTemplateArtifactDir::Fixed(output_dir.clone());

        let path = write_full_template_command_stdout_artifact(&artifact_dir, &output).unwrap();

        assert_eq!(fs::read(&path).unwrap(), output);
        assert_eq!(path.parent(), Some(output_dir.as_path()));
        let _ = fs::remove_dir_all(output_dir);
    }

    fn test_output_dir(label: &str) -> PathBuf {
        let random = getrandom::u64().unwrap();
        let path = std::env::temp_dir().join(format!(
            "canon-prompt-template-output-{label}-{}-{random:016x}",
            std::process::id()
        ));
        create_private_dir(&path).unwrap();
        path
    }
}
