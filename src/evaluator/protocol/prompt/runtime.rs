use super::super::prompt_artifact_permissions::{
    set_template_artifact_create_mode, set_template_artifact_file_permissions,
};
use super::super::prompt_shell::{
    quote_prompt_template_shell_arg, run_prompt_template_shell_command,
};
use crate::platform::{
    create_private_dir, memory_backed_temporary_parent_candidates,
    ordinary_temporary_parent_candidates,
};
use crate::process_cwd::with_current_dir;
use minijinja::value::{Kwargs, Value as MiniValue};
use minijinja::{Error, ErrorKind};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 8 * 1024;
pub(super) const PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX: &str = "canon-template-output";
const PROMPT_TEMPLATE_ARTIFACT_DIR_CREATE_ATTEMPTS: usize = 64;

#[derive(Clone)]
pub(super) struct ShTranscriptMarkers {
    pub(super) start: String,
    pub(super) end: String,
    pub(super) escape: String,
}

impl ShTranscriptMarkers {
    pub(super) fn new() -> Result<ShTranscriptMarkers, String> {
        let nonce = getrandom::u64()
            .map_err(|err| format!("failed to choose prompt template sentinel: {err}"))?;
        Ok(ShTranscriptMarkers {
            start: format!("\x1Fcanon-sh-transcript-start-{nonce:016x}\x1F"),
            end: format!("\x1Fcanon-sh-transcript-end-{nonce:016x}\x1F"),
            escape: format!("\x1Fcanon-sh-transcript-escape-{nonce:016x}\x1F"),
        })
    }

    pub(super) fn wrap_transcript(&self, transcript: &str) -> String {
        format!(
            "{}{}{}",
            self.start,
            encode_sh_transcript_marker_text(transcript, self),
            self.end
        )
    }
}

pub(super) fn trim_rendered_prompt_template_output(
    rendered: &str,
    sh_transcript_markers: &ShTranscriptMarkers,
) -> String {
    let mut output = String::new();
    let mut rest = rendered.trim();
    while let Some(start_index) = rest.find(&sh_transcript_markers.start) {
        output.push_str(&rest[..start_index]);
        let after_start = &rest[start_index + sh_transcript_markers.start.len()..];
        let Some(end_index) = after_start.find(&sh_transcript_markers.end) else {
            output.push_str(&rest[start_index..]);
            return output;
        };
        output.push_str(&decode_sh_transcript_marker_text(
            &after_start[..end_index],
            sh_transcript_markers,
        ));
        rest = &after_start[end_index + sh_transcript_markers.end.len()..];
    }
    output.push_str(rest);
    output
}

fn encode_sh_transcript_marker_text(transcript: &str, markers: &ShTranscriptMarkers) -> String {
    transcript
        .replace(&markers.escape, &(markers.escape.clone() + "e"))
        .replace(&markers.start, &(markers.escape.clone() + "s"))
        .replace(&markers.end, &(markers.escape.clone() + "n"))
}

fn decode_sh_transcript_marker_text(encoded: &str, markers: &ShTranscriptMarkers) -> String {
    let mut output = String::new();
    let mut rest = encoded;
    while let Some(index) = rest.find(&markers.escape) {
        output.push_str(&rest[..index]);
        rest = &rest[index + markers.escape.len()..];
        let Some(code) = rest.chars().next() else {
            output.push_str(&markers.escape);
            return output;
        };
        match code {
            'e' => output.push_str(&markers.escape),
            's' => output.push_str(&markers.start),
            'n' => output.push_str(&markers.end),
            _ => {
                output.push_str(&markers.escape);
                output.push(code);
            }
        }
        rest = &rest[code.len_utf8()..];
    }
    output.push_str(rest);
    output
}

pub(super) fn render_with_repository_cwd<F>(root: &Path, render: F) -> Result<String, Error>
where
    F: FnOnce() -> Result<String, Error>,
{
    with_current_dir(root, render).map_err(template_error)?
}

pub(super) struct PromptTemplateOutputDirCache {
    dir: OnceLock<PromptTemplateOutputDir>,
}

#[derive(Clone)]
pub(super) enum PromptTemplateArtifactDir {
    Lazy(Arc<PromptTemplateOutputDirCache>),
    #[cfg(test)]
    Fixed(PathBuf),
}

impl PromptTemplateArtifactDir {
    fn path(&self) -> Result<PathBuf, String> {
        match self {
            PromptTemplateArtifactDir::Lazy(cache) => cache.path_for_prompt_artifacts(),
            #[cfg(test)]
            PromptTemplateArtifactDir::Fixed(path) => Ok(path.clone()),
        }
    }
}

impl PromptTemplateOutputDirCache {
    pub(super) fn new() -> PromptTemplateOutputDirCache {
        PromptTemplateOutputDirCache {
            dir: OnceLock::new(),
        }
    }

    pub(super) fn path_for_prompt_artifacts(&self) -> Result<PathBuf, String> {
        if let Some(dir) = self.dir.get() {
            return Ok(dir.path().to_path_buf());
        }
        let dir = allocate_prompt_template_artifact_dir()?;
        let path = dir.path().to_path_buf();
        if self.dir.set(dir).is_err() {
            return Ok(self
                .dir
                .get()
                .expect("prompt template output dir is set")
                .path()
                .to_path_buf());
        }
        Ok(path)
    }
}

pub(super) struct PromptTemplateOutputDir {
    path: PathBuf,
}

impl PromptTemplateOutputDir {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PromptTemplateOutputDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn allocate_prompt_template_artifact_dir() -> Result<PromptTemplateOutputDir, String> {
    // Prompt Templates require oversized command output to be exposed through
    // evaluator-readable temp files. This allocator returns a private artifact
    // directory; `PromptTemplateOutputDirCache` shares one directory across the
    // prompt renders that may need to reference those transcript artifacts.
    let memory_backed_candidates = memory_backed_temporary_parent_candidates();
    let fallback_candidates = ordinary_temporary_parent_candidates();
    allocate_prompt_template_artifact_dir_from_candidates(
        &memory_backed_candidates,
        &fallback_candidates,
    )
}

pub(super) fn allocate_prompt_template_artifact_dir_from_candidates(
    memory_backed_candidates: &[PathBuf],
    fallback_candidates: &[PathBuf],
) -> Result<PromptTemplateOutputDir, String> {
    // Canon-owned temporary directories prefer memory-backed parents when the
    // host provides one; ordinary temporary parents are only the fallback path.
    let mut errors = Vec::new();
    for parent in memory_backed_candidates
        .iter()
        .chain(fallback_candidates.iter())
    {
        match allocate_prompt_template_artifact_dir_in(parent) {
            Ok(dir) => return Ok(dir),
            Err(err) => errors.push(err),
        }
    }
    Err(format!(
        "failed to allocate a unique prompt template output directory: {}",
        errors.join("; ")
    ))
}

fn allocate_prompt_template_artifact_dir_in(
    parent: &Path,
) -> Result<PromptTemplateOutputDir, String> {
    if !parent.is_dir() {
        return Err(format!("{} is not a directory", parent.display()));
    }
    for _ in 0..PROMPT_TEMPLATE_ARTIFACT_DIR_CREATE_ATTEMPTS {
        let random = getrandom::u64()
            .map_err(|err| format!("failed to choose prompt template output directory: {err}"))?;
        let path = parent.join(format!(
            "{}-{}-{random:016x}",
            PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX,
            std::process::id()
        ));
        match create_private_dir(&path) {
            Ok(()) => return Ok(PromptTemplateOutputDir { path }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create {}: {}", path.display(), err)),
        }
    }
    Err(format!(
        "failed to allocate a unique prompt template output directory under {}",
        parent.display()
    ))
}

pub(super) fn json_filter(value: MiniValue) -> Result<String, Error> {
    serde_json::to_string(&value).map_err(|err| template_error(err.to_string()))
}

pub(super) fn shell_quote_filter(value: String) -> Result<String, Error> {
    quote_prompt_template_shell_arg(&value).map_err(template_error)
}

pub(super) fn shell_args_filter(value: MiniValue) -> Result<String, Error> {
    value
        .try_iter()
        .map_err(|err| template_error(format!("shargs requires an iterable: {err}")))?
        .map(|arg| {
            let arg = arg
                .as_str()
                .ok_or_else(|| template_error("shargs requires string arguments".to_string()))?;
            quote_prompt_template_shell_arg(arg).map_err(template_error)
        })
        .collect::<Result<Vec<_>, Error>>()
        .map(|args| args.join(" "))
}

pub(super) fn shell_transcript_filter(
    root: &Path,
    template_artifact_dir: &PromptTemplateArtifactDir,
    template_artifact_paths: &Mutex<Vec<PathBuf>>,
    template_shell_env: &[(OsString, OsString)],
    template_shell_args: &[String],
    command: String,
    kwargs: Kwargs,
) -> Result<String, Error> {
    let command = command.trim().to_string();
    let display = kwargs
        .get::<Option<String>>("display")
        .map_err(|err| template_error(err.to_string()))?
        .unwrap_or_else(|| command.clone());
    kwargs
        .assert_all_used()
        .map_err(|err| template_error(err.to_string()))?;
    // The prompt-template `sh` filter is defined to run the rendered block body
    // as a shell command. That CWD-sensitive template operation runs from the
    // repository root without mutating the parent process cwd.
    let output =
        run_prompt_template_shell_command(root, &command, template_shell_env, template_shell_args)
            .map_err(|err| {
                template_error(format!("failed to run prompt template command: {err}"))
            })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(template_error(format!(
            "prompt template command failed: {}",
            stderr.trim()
        )));
    }
    let mut transcript = String::new();
    transcript.push_str("$ ");
    transcript.push_str(&display);
    transcript.push('\n');
    // The transcript shape is exactly command line, command stdout, and
    // optionally the single truncation marker appended below; no extra
    // begin/end sentinel lines are part of the Prompt Templates contract.
    transcript.push_str(&truncated_template_command_output(
        &output.stdout,
        template_artifact_dir,
        template_artifact_paths,
    )?);
    if !transcript.ends_with('\n') {
        transcript.push('\n');
    }
    Ok(transcript)
}

pub(super) fn truncated_template_command_output(
    stdout: &[u8],
    template_artifact_dir: &PromptTemplateArtifactDir,
    template_artifact_paths: &Mutex<Vec<PathBuf>>,
) -> Result<String, Error> {
    if stdout.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(String::from_utf8_lossy(stdout).into_owned());
    }
    // Prompt-template truncation is a token-budget mechanism, not a visibility
    // boundary. The Prompt Templates spec deliberately exposes a readable file
    // containing the same command stdout so the evaluator can inspect more of
    // an already-visible transcript when the head is insufficient. Canon never
    // reads this artifact back as check-run state.
    let template_output_dir = template_artifact_dir.path().map_err(template_error)?;
    let path = write_full_template_command_stdout_artifact(&template_output_dir, stdout)?;
    template_artifact_paths.lock().unwrap().push(path.clone());
    let (mut head, head_lines) = template_command_stdout_head(stdout);
    if !head.ends_with('\n') {
        head.push('\n');
    }
    let total_lines = output_line_count(stdout);
    head.push_str(&format!(
        "[truncated: showing first {} of {} lines; full output: {}]\n",
        head_lines,
        total_lines,
        path.display()
    ));
    Ok(head)
}

fn template_command_stdout_head(stdout: &[u8]) -> (String, usize) {
    let mut end = 0usize;
    let mut head_lines = 0usize;
    for line in stdout.split_inclusive(|byte| *byte == b'\n') {
        if end.saturating_add(line.len()) > TEMPLATE_OUTPUT_HEAD_BYTES {
            break;
        }
        end += line.len();
        head_lines += 1;
    }
    if head_lines > 0 {
        return (
            String::from_utf8_lossy(&stdout[..end]).into_owned(),
            head_lines,
        );
    }

    let end = TEMPLATE_OUTPUT_HEAD_BYTES.min(stdout.len());
    let head = String::from_utf8_lossy(&stdout[..end]).into_owned();
    // No complete line fit in the byte budget. The byte head remains useful,
    // but it must not be reported as one fully shown line.
    (head, 0)
}

fn output_line_count(output: &[u8]) -> usize {
    if output.is_empty() {
        return 0;
    }
    let lines = output.iter().filter(|byte| **byte == b'\n').count();
    if output.ends_with(b"\n") {
        lines
    } else {
        lines + 1
    }
}

fn write_full_template_command_stdout_artifact(
    template_output_dir: &Path,
    stdout: &[u8],
) -> Result<PathBuf, Error> {
    let path = template_command_stdout_artifact_path(template_output_dir, stdout);
    // This artifact is part of the evaluator-readable prompt transcript, not
    // canon check state. The implementation writes it so the truncation line's
    // path is readable by the evaluator, and never reads it back to make
    // check-run decisions.
    match create_template_command_stdout_artifact_file(&path, stdout) {
        Ok(()) => Ok(path),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read(&path).map_err(|read_err| {
                template_error(format!(
                    "failed to read existing prompt template output file {}: {}",
                    path.display(),
                    read_err
                ))
            })?;
            if existing == stdout {
                return Ok(path);
            }
            Err(template_error(format!(
                "prompt template output hash collision or stale file at {}",
                path.display()
            )))
        }
        Err(err) => Err(template_error(format!(
            "failed to write {}: {}",
            path.display(),
            err
        ))),
    }
}

pub(super) fn template_command_stdout_artifact_path(
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

fn template_error(message: String) -> Error {
    Error::new(ErrorKind::InvalidOperation, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::create_private_dir;

    #[test] // xpec: C
    fn template_command_output_truncates_large_output() {
        let output = (0..6000)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("truncate");
        let artifact_paths = Mutex::new(Vec::new());
        let rendered = truncated_template_command_output(
            output.as_bytes(),
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();

        assert!(rendered.contains("[truncated: showing first "));
        assert!(rendered.contains("; full output: "));
        assert!(!rendered.contains("[begin untrusted command output"));
        assert!(!rendered.contains("[end untrusted command output"));
        assert_eq!(artifact_paths.lock().unwrap().len(), 1);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: C
    fn partial_oversized_first_line_is_not_counted_as_shown() {
        let output = vec![b'x'; TEMPLATE_OUTPUT_HEAD_BYTES + 1];
        let output_dir = test_output_dir("partial-first-line");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered = truncated_template_command_output(
            &output,
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();

        assert!(rendered.contains("[truncated: showing first 0 of 1 lines; full output: "));
        let path = full_output_path_from_rendered(&rendered);
        assert_eq!(fs::read(path).unwrap(), output);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: C,dx
    fn template_command_output_file_is_content_addressed_and_deduplicated() {
        let output = (0..1200)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("dedupe");
        let artifact_paths = Mutex::new(Vec::new());

        let first = truncated_template_command_output(
            output.as_bytes(),
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();
        let second = truncated_template_command_output(
            output.as_bytes(),
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();
        let first_path = PathBuf::from(full_output_path_from_rendered(&first));
        let second_path = PathBuf::from(full_output_path_from_rendered(&second));

        assert_eq!(first_path, second_path);
        assert_eq!(
            artifact_paths.lock().unwrap().as_slice(),
            &[first_path.clone(), first_path.clone()]
        );
        assert!(first_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("canon-template-output-sha256-"));
        assert_eq!(fs::read(&first_path).unwrap(), output.as_bytes());
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: C,M
    fn prompt_template_output_dir_allocations_are_fresh() {
        let first = allocate_prompt_template_artifact_dir().unwrap();
        let second = allocate_prompt_template_artifact_dir().unwrap();

        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
        for path in [first.path(), second.path()] {
            assert!(path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX));
        }
    }

    #[test] // xpec: M
    fn prompt_template_output_dir_does_not_reuse_fixed_temp_path() {
        let fixed = std::env::temp_dir().join(PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX);

        let output = allocate_prompt_template_artifact_dir().unwrap();

        assert_ne!(output.path(), fixed);
        assert!(output.path().is_dir());
    }

    #[test] // xpec: M
    fn prompt_template_output_dir_prefers_memory_backed_parent() {
        let memory_backed_parent = test_output_dir("memory-backed-parent");
        let fallback_parent = test_output_dir("fallback-parent");
        let memory_backed_candidates = vec![memory_backed_parent.clone()];
        let fallback_candidates = vec![fallback_parent.clone()];

        let output = allocate_prompt_template_artifact_dir_from_candidates(
            &memory_backed_candidates,
            &fallback_candidates,
        )
        .unwrap();
        let output_path = output.path().to_path_buf();

        assert!(output_path.starts_with(&memory_backed_parent));
        assert!(!output_path.starts_with(&fallback_parent));
        drop(output);
        let _ = fs::remove_dir_all(memory_backed_parent);
        let _ = fs::remove_dir_all(fallback_parent);
    }

    #[test] // xpec: M
    fn prompt_template_output_dir_falls_back_when_memory_backed_parent_is_unavailable() {
        let missing_parent = std::env::temp_dir().join(format!(
            "canon-missing-memory-backed-parent-{}",
            std::process::id()
        ));
        let fallback_parent = test_output_dir("fallback-parent");
        let memory_backed_candidates = vec![missing_parent];
        let fallback_candidates = vec![fallback_parent.clone()];

        let output = allocate_prompt_template_artifact_dir_from_candidates(
            &memory_backed_candidates,
            &fallback_candidates,
        )
        .unwrap();
        let output_path = output.path().to_path_buf();

        assert!(output_path.starts_with(&fallback_parent));
        drop(output);
        let _ = fs::remove_dir_all(fallback_parent);
    }

    #[test] // xpec: C,dx
    fn prompt_template_output_dir_cache_reuses_artifact_dir() {
        let first;
        {
            let cache = PromptTemplateOutputDirCache::new();

            first = cache.path_for_prompt_artifacts().unwrap();
            let second = cache.path_for_prompt_artifacts().unwrap();

            assert_eq!(first, second);
            assert!(first.is_dir());
        }
        assert!(!first.exists());
    }

    #[test] // xpec: C
    fn prompt_template_output_dir_caches_use_distinct_artifact_dirs() {
        let first;
        let second;
        {
            let first_cache = PromptTemplateOutputDirCache::new();
            let second_cache = PromptTemplateOutputDirCache::new();

            first = first_cache.path_for_prompt_artifacts().unwrap();
            second = second_cache.path_for_prompt_artifacts().unwrap();

            assert_ne!(first, second);
            assert!(first.is_dir());
            assert!(second.is_dir());
        }
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[test] // xpec: C
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

    #[test] // xpec: C
    fn template_command_output_file_preserves_raw_stdout_bytes() {
        // The saved file is raw command stdout. The full rendered prompt string
        // is trimmed separately after all `sh` filters return.
        let mut output = (0..1200)
            .flat_map(|index| format!("line {index}\n").into_bytes())
            .collect::<Vec<_>>();
        output.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let output_dir = test_output_dir("raw-bytes");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered = truncated_template_command_output(
            &output,
            &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &artifact_paths,
        )
        .unwrap();
        let path = full_output_path_from_rendered(&rendered);
        let saved = fs::read(path).unwrap();

        assert_eq!(saved, output);
        assert_eq!(
            artifact_paths.lock().unwrap().as_slice(),
            &[PathBuf::from(path)]
        );
        let saved_path = Path::new(path);
        assert_eq!(saved_path.parent(), Some(output_dir.as_path()));
        let _ = fs::remove_dir_all(output_dir);
    }

    fn full_output_path_from_rendered(rendered: &str) -> &str {
        rendered
            .lines()
            .find(|line| line.starts_with("[truncated: "))
            .unwrap()
            .strip_suffix(']')
            .unwrap()
            .rsplit_once("full output: ")
            .unwrap()
            .1
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

    #[test] // xpec: C
    fn outer_trim_preserves_shell_transcript_edges() {
        let markers = test_markers();
        let transcript = "$ cmd\n  output  \n";
        let rendered = format!("\n  {}  \n", markers.wrap_transcript(transcript));

        let trimmed = trim_rendered_prompt_template_output(&rendered, &markers);

        assert_eq!(trimmed, transcript);
    }

    #[test] // xpec: C
    fn shell_transcript_markers_are_preserved_inside_transcript_text() {
        let markers = test_markers();
        let transcript = format!(
            "$ cmd\n{}{}{}\n",
            markers.start, markers.end, markers.escape
        );
        let rendered = markers.wrap_transcript(&transcript);

        let trimmed = trim_rendered_prompt_template_output(&rendered, &markers);

        assert_eq!(trimmed, transcript);
    }

    fn test_markers() -> ShTranscriptMarkers {
        ShTranscriptMarkers {
            start: "<start>".to_string(),
            end: "<end>".to_string(),
            escape: "<escape>".to_string(),
        }
    }
}
