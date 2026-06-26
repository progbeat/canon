use super::prompt_shell::{quote_prompt_template_shell_arg, run_prompt_template_shell_command};
use crate::platform::create_private_dir;
use crate::process_cwd::with_current_dir;
use crate::xpec_state::LastResult;
use minijinja::value::{Kwargs, Value as MiniValue};
use minijinja::{Environment, Error, ErrorKind};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 8 * 1024;
const PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX: &str = "canon-template-output";
const PROMPT_TEMPLATE_ARTIFACT_DIR_CREATE_ATTEMPTS: usize = 64;

// Canon-owned evaluator templates are loaded from `resources/prompts/`; this
// module only renders those resource files with runtime check data.
const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_developer_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_turn_prompt.txt");

pub(crate) struct DeveloperInstructionsContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) template_output_dir: &'a Path,
    pub(crate) template_artifact_paths: &'a mut Vec<PathBuf>,
    pub(crate) in_place: bool,
    pub(crate) diff_from_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
    // Data for the resource template's `xpec.instructions` variable.
    pub(crate) question_context: &'a str,
    pub(crate) q_scope: &'a [String],
    pub(crate) ignore: &'a [String],
    pub(crate) visible_scope: &'a [String],
    pub(crate) checked_file_count: usize,
    pub(crate) visible_file_count: usize,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn developer_instructions(
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    // This count is reporting-only prompt data. File visibility has already
    // been decided by the visible-tree pathspec selection in
    // `src/git/visible_tree_oid/`; the template's "likely unnecessary" wording
    // does not add another hiding rule.
    let files_not_selected_by_visible_scope_pathspec = context
        .checked_file_count
        .checked_sub(context.visible_file_count)
        .ok_or("visible file count exceeds checked file count")?;
    // The transcript intentionally has two scoped diff views over
    // `visible_scope`: `git diff --numstat` for change discovery, then detailed
    // `git diff` for inspectable content. Template display text omits the
    // pathspec so developer instructions show the relevant tree OIDs without
    // repeating noisy scope arguments.
    render_minijinja_resource_template(
        context.root,
        context.template_output_dir,
        context.template_artifact_paths,
        DEVELOPER_INSTRUCTIONS_TEMPLATE,
        &[
            ("BASE_TREE", context.diff_from_tree_oid),
            ("CHECKED_TREE", context.checked_tree_oid),
        ],
        json!({
            "xpec": {
                "instructions": context.question_context,
                "q_scope": context.q_scope,
                "ignore": context.ignore,
                "visible_scope": context.visible_scope,
            },
            "in_place": context.in_place,
            "last_pass": context.last_pass,
            "num_invisible_files": files_not_selected_by_visible_scope_pathspec,
        }),
    )
}

pub(crate) struct EvaluatorTurnPromptContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) template_output_dir: &'a Path,
    pub(crate) template_artifact_paths: &'a mut Vec<PathBuf>,
    pub(crate) short_id: &'a str,
    pub(crate) question: &'a str,
    pub(crate) expected_answer: &'a str,
    pub(crate) in_place: bool,
    pub(crate) diff_from: &'a str,
    pub(crate) target: Option<&'a str>,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn evaluator_turn_prompt(
    context: EvaluatorTurnPromptContext<'_>,
) -> Result<String, String> {
    let (diff_from, target, last_pass) = if context.in_place {
        // In-place mode has no Git diff target or checkpoint context. The
        // caller validates selected expectations before interrogation; this
        // clamp keeps the rendered prompt diff-free even if an invalid in-place
        // expectation reaches this component.
        ("", None, None)
    } else {
        (context.diff_from, context.target, context.last_pass)
    };
    // `diff_from` is template input for this fresh evaluator turn only. Cached
    // results are emitted without rendering this prompt. The turn template uses
    // `xpec.diff_from` to choose whether a target-diff prompt can reuse the
    // checkpoint response or must render the xpec's default answer.
    // `target` is the same kind of per-turn prompt input; it is deliberately
    // not part of evaluator thread reuse.
    let xpec_context = turn_prompt_xpec_context(
        context.short_id,
        context.question,
        context.expected_answer,
        diff_from,
        target,
    );
    render_minijinja_resource_template(
        context.root,
        context.template_output_dir,
        context.template_artifact_paths,
        EVALUATOR_TURN_PROMPT_TEMPLATE,
        &[],
        json!({
            "xpec": xpec_context,
            "last_pass": last_pass,
        }),
    )
}

fn turn_prompt_xpec_context(
    short_id: &str,
    question: &str,
    expected_answer: &str,
    diff_from: &str,
    target: Option<&str>,
) -> JsonValue {
    json!({
        "short_id": short_id,
        "q": question,
        "a": expected_answer,
        "diff_from": diff_from,
        "target": target.unwrap_or(""),
    })
}

fn render_minijinja_resource_template(
    root: &Path,
    template_output_dir: &Path,
    template_artifact_paths: &mut Vec<PathBuf>,
    template: &str,
    template_shell_env: &[(&str, &str)],
    context: JsonValue,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    environment.add_filter("shargs", shell_args_filter);
    let command_root = root.to_path_buf();
    let command_output_dir = template_output_dir.to_path_buf();
    let command_env = template_shell_env
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<Vec<_>>();
    let command_artifact_paths = Arc::new(Mutex::new(Vec::new()));
    let sh_transcript_markers = ShTranscriptMarkers::new()?;
    let filter_transcript_markers = sh_transcript_markers.clone();
    let filter_artifact_paths = Arc::clone(&command_artifact_paths);
    environment.add_filter(
        "sh",
        move |command: String, kwargs: Kwargs| -> Result<String, Error> {
            let transcript = shell_transcript_filter(
                &command_root,
                &command_output_dir,
                filter_artifact_paths.as_ref(),
                &command_env,
                command,
                kwargs,
            )?;
            Ok(filter_transcript_markers.wrap_transcript(&transcript))
        },
    );
    let template = environment
        .template_from_str(template)
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    // Prompt Templates require the MiniJinja render itself to start from this
    // check root cwd: the repository root outside in-place mode, or the
    // checked directory in in-place mode.
    let rendered = render_with_repository_cwd(root, || template.render(context))
        .map_err(|err| format!("failed to render prompt template: {}", err))?;
    template_artifact_paths.extend(command_artifact_paths.lock().unwrap().iter().cloned());
    // This is the final rendered prompt trim required by Prompt Templates. It
    // is separate from `sh` command-body trimming in `shell_transcript_filter`.
    // Internal sentinels protect `sh` transcript text at prompt boundaries so
    // command display text, stdout text, saved stdout bytes, and the
    // truncation marker keep their specified spelling.
    Ok(trim_rendered_prompt_template_output(
        &rendered,
        &sh_transcript_markers,
    ))
}

#[derive(Clone)]
struct ShTranscriptMarkers {
    start: String,
    end: String,
    escape: String,
}

impl ShTranscriptMarkers {
    fn new() -> Result<ShTranscriptMarkers, String> {
        let nonce = getrandom::u64()
            .map_err(|err| format!("failed to choose prompt template sentinel: {err}"))?;
        Ok(ShTranscriptMarkers {
            start: format!("\x1Fcanon-sh-transcript-start-{nonce:016x}\x1F"),
            end: format!("\x1Fcanon-sh-transcript-end-{nonce:016x}\x1F"),
            escape: format!("\x1Fcanon-sh-transcript-escape-{nonce:016x}\x1F"),
        })
    }

    fn wrap_transcript(&self, transcript: &str) -> String {
        format!(
            "{}{}{}",
            self.start,
            encode_sh_transcript_marker_text(transcript, self),
            self.end
        )
    }
}

fn trim_rendered_prompt_template_output(
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

fn render_with_repository_cwd<F>(root: &Path, render: F) -> Result<String, Error>
where
    F: FnOnce() -> Result<String, Error>,
{
    with_current_dir(root, render).map_err(template_error)?
}

pub(crate) struct PromptTemplateOutputDirCache {
    dir: OnceLock<PromptTemplateOutputDir>,
}

impl PromptTemplateOutputDirCache {
    pub(crate) fn new() -> PromptTemplateOutputDirCache {
        PromptTemplateOutputDirCache {
            dir: OnceLock::new(),
        }
    }

    pub(crate) fn path_for_check_invocation(&self) -> Result<PathBuf, String> {
        if let Some(dir) = self.dir.get() {
            return Ok(dir.path().to_path_buf());
        }
        let dir = allocate_prompt_template_output_dir_for_check_invocation()?;
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

struct PromptTemplateOutputDir {
    path: PathBuf,
}

impl PromptTemplateOutputDir {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PromptTemplateOutputDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn allocate_prompt_template_output_dir_for_check_invocation(
) -> Result<PromptTemplateOutputDir, String> {
    // This allocator intentionally returns a fresh private directory per call.
    // Production check runs call it through `PromptTemplateOutputDirCache`,
    // which caches one returned path for every prompt render in that invocation.
    let parent = std::env::temp_dir();
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

fn json_filter(value: MiniValue) -> Result<String, Error> {
    serde_json::to_string(&value).map_err(|err| template_error(err.to_string()))
}

fn shell_quote_filter(value: String) -> Result<String, Error> {
    quote_prompt_template_shell_arg(&value).map_err(template_error)
}

fn shell_args_filter(value: MiniValue) -> Result<String, Error> {
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

fn shell_transcript_filter(
    root: &Path,
    template_output_dir: &Path,
    template_artifact_paths: &Mutex<Vec<PathBuf>>,
    template_shell_env: &[(String, String)],
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
    let env_refs = template_shell_env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let output = run_prompt_template_shell_command(root, &command, &env_refs)
        .map_err(|err| template_error(format!("failed to run prompt template command: {err}")))?;
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
        template_output_dir,
        template_artifact_paths,
    )?);
    if !transcript.ends_with('\n') {
        transcript.push('\n');
    }
    Ok(transcript)
}

fn truncated_template_command_output(
    stdout: &[u8],
    template_output_dir: &Path,
    template_artifact_paths: &Mutex<Vec<PathBuf>>,
) -> Result<String, Error> {
    if stdout.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(String::from_utf8_lossy(stdout).into_owned());
    }
    // Prompt-template truncation is a token-budget mechanism, not a visibility
    // boundary. The Prompt Templates spec deliberately exposes a readable file
    // containing the same command stdout so the evaluator can inspect more of
    // an already-visible transcript when the head is insufficient. Canon never
    // reads this artifact back as invocation state.
    let path = write_full_template_command_stdout_artifact(template_output_dir, stdout)?;
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
    let head_lines = output_line_count(&stdout[..end]);
    (head, head_lines)
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

fn template_command_stdout_artifact_path(template_output_dir: &Path, stdout: &[u8]) -> PathBuf {
    // `template_output_dir` is stable for one check run, and the file name is
    // content-addressed by complete stdout bytes. Identical stdout therefore
    // maps to the same full path within that `canon check` invocation.
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

#[cfg(unix)]
fn set_template_artifact_create_mode(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_template_artifact_create_mode(_options: &mut fs::OpenOptions) {}

#[cfg(unix)]
fn set_template_artifact_file_permissions(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_template_artifact_file_permissions(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

fn template_error(message: String) -> Error {
    Error::new(ErrorKind::InvalidOperation, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpec_state::LastResultStatus;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn developer_instructions_include_transcript_outside_in_place_mode() {
        let rendered = developer_instructions_for_mode(false);

        assert!(rendered.contains("Use the transcript below only for context/navigation"));
        assert!(rendered.contains("$ git diff --numstat $BASE_TREE $CHECKED_TREE"));
        assert!(rendered.contains("$ git diff $BASE_TREE $CHECKED_TREE"));
        assert!(rendered.contains("$ enter-sandbox --scope [\"src\"] --ignore []"));
    }

    #[test]
    fn developer_instructions_omit_transcript_in_in_place_mode() {
        let rendered = developer_instructions_for_mode(true);

        assert!(rendered.contains("Custom expectation instructions."));
        assert!(!rendered.contains("Use the transcript below only for context/navigation"));
        assert!(!rendered.contains("$ git diff --numstat"));
        assert!(!rendered.contains("$ git diff"));
        assert!(!rendered.contains("$ enter-sandbox"));
    }

    #[test]
    fn template_command_output_truncates_large_output() {
        let output = (0..6000)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("truncate");
        let artifact_paths = Mutex::new(Vec::new());
        let rendered =
            truncated_template_command_output(output.as_bytes(), &output_dir, &artifact_paths)
                .unwrap();

        assert!(rendered.contains("[truncated: showing first "));
        assert!(rendered.contains("; full output: "));
        assert!(!rendered.contains("[begin untrusted command output"));
        assert!(!rendered.contains("[end untrusted command output"));
        assert_eq!(artifact_paths.lock().unwrap().len(), 1);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn template_command_output_file_is_content_addressed_and_deduplicated() {
        let output = (0..1200)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("dedupe");
        let artifact_paths = Mutex::new(Vec::new());

        let first =
            truncated_template_command_output(output.as_bytes(), &output_dir, &artifact_paths)
                .unwrap();
        let second =
            truncated_template_command_output(output.as_bytes(), &output_dir, &artifact_paths)
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

    #[test]
    fn sh_transcript_boundary_whitespace_survives_outer_trim() {
        let output_dir = test_output_dir("sh-boundary-trim");
        let mut artifact_paths = Vec::new();

        let rendered = render_minijinja_resource_template(
            Path::new("."),
            &output_dir,
            &mut artifact_paths,
            " \n{% filter sh(display=\"printf kept\") %}printf '  kept\\n'{% endfilter %}\n ",
            &[],
            json!({}),
        )
        .unwrap();

        assert_eq!(rendered, "$ printf kept\n  kept\n");
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn prompt_template_output_dir_allocations_are_fresh() {
        let first = allocate_prompt_template_output_dir_for_check_invocation().unwrap();
        let second = allocate_prompt_template_output_dir_for_check_invocation().unwrap();

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

    #[test]
    fn prompt_template_output_dir_does_not_reuse_fixed_temp_path() {
        let fixed = std::env::temp_dir().join(PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX);

        let output = allocate_prompt_template_output_dir_for_check_invocation().unwrap();

        assert_ne!(output.path(), fixed);
        assert!(output.path().is_dir());
    }

    #[test]
    fn prompt_template_output_dir_cache_is_stable_within_invocation() {
        let first;
        {
            let cache = PromptTemplateOutputDirCache::new();

            first = cache.path_for_check_invocation().unwrap();
            let second = cache.path_for_check_invocation().unwrap();

            assert_eq!(first, second);
            assert!(first.is_dir());
        }
        assert!(!first.exists());
    }

    #[test]
    fn prompt_template_output_dir_caches_are_fresh_per_invocation() {
        let first;
        let second;
        {
            let first_cache = PromptTemplateOutputDirCache::new();
            let second_cache = PromptTemplateOutputDirCache::new();

            first = first_cache.path_for_check_invocation().unwrap();
            second = second_cache.path_for_check_invocation().unwrap();

            assert_ne!(first, second);
            assert!(first.is_dir());
            assert!(second.is_dir());
        }
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[test]
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

    #[test]
    fn template_command_output_file_preserves_raw_stdout_bytes() {
        // The saved file is raw command stdout. The full rendered prompt string
        // is trimmed separately after all `sh` filters return.
        let mut output = (0..1200)
            .flat_map(|index| format!("line {index}\n").into_bytes())
            .collect::<Vec<_>>();
        output.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let output_dir = test_output_dir("raw-bytes");
        let artifact_paths = Mutex::new(Vec::new());

        let rendered =
            truncated_template_command_output(&output, &output_dir, &artifact_paths).unwrap();
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

    fn developer_instructions_for_mode(in_place: bool) -> String {
        let output_dir = test_output_dir(if in_place {
            "developer-instructions-in-place"
        } else {
            "developer-instructions-normal"
        });
        let mut artifact_paths = Vec::new();
        let visible_scope = vec!["src".to_string()];
        let ignore = Vec::new();
        let rendered = developer_instructions(DeveloperInstructionsContext {
            root: Path::new("."),
            template_output_dir: &output_dir,
            template_artifact_paths: &mut artifact_paths,
            in_place,
            diff_from_tree_oid: "HEAD",
            checked_tree_oid: "HEAD",
            question_context: "Custom expectation instructions.",
            q_scope: &visible_scope,
            ignore: &ignore,
            visible_scope: &visible_scope,
            checked_file_count: 10,
            visible_file_count: 5,
            last_pass: None,
        })
        .unwrap();
        let _ = fs::remove_dir_all(output_dir);
        rendered
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

    #[test]
    fn target_diff_prompt_hint_uses_full_q_scope_suggestion() {
        let last_pass = LastResult {
            response_timestamp: "1970-01-01T00:00:01Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
            status: LastResultStatus::Pass,
            response: json!({
                "answer": "yes",
                "evidence": "`src/a.rs`",
                "qScopeSuggestion": ["src/a.rs"],
            }),
            q_scope: vec!["src/a.rs".to_string()],
            visible_scope: vec!["src/a.rs".to_string()],
            checked_tree_oid: Some("checked-tree".to_string()),
            visible_tree_oid: Some("visible-tree".to_string()),
        };
        let output_dir = test_output_dir("turn-prompt");
        let mut artifact_paths = Vec::new();

        let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
            root: Path::new("."),
            template_output_dir: &output_dir,
            template_artifact_paths: &mut artifact_paths,
            short_id: "e",
            question: "Does it pass?",
            expected_answer: "yes",
            in_place: false,
            diff_from: crate::config_types::DEFAULT_DIFF_FROM,
            target: Some("diff"),
            last_pass: Some(&last_pass),
        })
        .unwrap();

        assert!(prompt.contains("This question targets the Git diff."));
        assert!(prompt.contains("Use this prior evaluation if it still holds:"));
        assert!(prompt.contains(r#"{"e":"Does it pass?"}"#));
        assert!(prompt.contains(r#""answer":"yes""#));
        assert!(prompt.contains(r#""evidence":"`src/a.rs`""#));
        // The turn prompt provides this response literal to the evaluator. The
        // base instruction to keep a provided response's qScopeSuggestion
        // refers to this rendered literal, not the stored last-pass response
        // that was used as template input.
        assert!(prompt.contains(r#""qScopeSuggestion":["."]"#));
        assert!(!prompt.contains(r#""qScopeSuggestion":["src/a.rs"]"#));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn target_diff_prompt_uses_expected_answer_when_diff_from_is_not_checkpoint() {
        let last_pass = LastResult {
            response_timestamp: "1970-01-01T00:00:01Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
            status: LastResultStatus::Pass,
            response: json!({
                "answer": "no",
                "evidence": "`src/a.rs`",
                "qScopeSuggestion": ["src/a.rs"],
            }),
            q_scope: vec!["src/a.rs".to_string()],
            visible_scope: vec!["src/a.rs".to_string()],
            checked_tree_oid: Some("checked-tree".to_string()),
            visible_tree_oid: Some("visible-tree".to_string()),
        };
        let output_dir = test_output_dir("turn-prompt-against-tree");
        let mut artifact_paths = Vec::new();

        let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
            root: Path::new("."),
            template_output_dir: &output_dir,
            template_artifact_paths: &mut artifact_paths,
            short_id: "e",
            question: "Does it pass?",
            expected_answer: "yes",
            in_place: false,
            diff_from: crate::config_types::AGAINST_TREE_DIFF_FROM,
            target: Some("diff"),
            last_pass: Some(&last_pass),
        })
        .unwrap();

        assert!(prompt.contains("This question targets the Git diff."));
        assert!(prompt.contains(r#""evidence":"""#));
        assert!(prompt.contains(r#""answer":"yes""#));
        assert!(!prompt.contains(r#""answer":"no""#));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn in_place_turn_prompt_omits_target_diff_hint() {
        let last_pass = LastResult {
            response_timestamp: "1970-01-01T00:00:01Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
            status: LastResultStatus::Pass,
            response: json!({
                "answer": "yes",
                "evidence": "`src/a.rs`",
                "qScopeSuggestion": ["src/a.rs"],
            }),
            q_scope: vec!["src/a.rs".to_string()],
            visible_scope: vec!["src/a.rs".to_string()],
            checked_tree_oid: Some("checked-tree".to_string()),
            visible_tree_oid: Some("visible-tree".to_string()),
        };
        let output_dir = test_output_dir("turn-prompt-in-place");
        let mut artifact_paths = Vec::new();

        let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
            root: Path::new("."),
            template_output_dir: &output_dir,
            template_artifact_paths: &mut artifact_paths,
            short_id: "e",
            question: "Does it pass?",
            expected_answer: "yes",
            in_place: true,
            diff_from: crate::config_types::DEFAULT_DIFF_FROM,
            target: Some("diff"),
            last_pass: Some(&last_pass),
        })
        .unwrap();

        assert_eq!(prompt, r#"{"e":"Does it pass?"}"#);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resource_template_rendering_trims_outer_whitespace() {
        let output_dir = test_output_dir("outer-trim");
        let mut artifact_paths = Vec::new();

        let rendered = render_minijinja_resource_template(
            Path::new("."),
            &output_dir,
            &mut artifact_paths,
            "\n  {{ value }}  \n",
            &[],
            json!({ "value": "answer" }),
        )
        .unwrap();

        assert_eq!(rendered, "answer");
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn outer_trim_preserves_shell_transcript_edges() {
        let markers = test_markers();
        let transcript = "$ cmd\n  output  \n";
        let rendered = format!("\n  {}  \n", markers.wrap_transcript(transcript));

        let trimmed = trim_rendered_prompt_template_output(&rendered, &markers);

        assert_eq!(trimmed, transcript);
    }

    #[test]
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
