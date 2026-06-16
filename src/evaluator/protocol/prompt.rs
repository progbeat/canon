use super::prompt_shell::{quote_prompt_template_shell_arg, run_prompt_template_shell_command};
use crate::platform::create_private_dir;
use crate::process_cwd::with_current_dir;
use crate::xpec_state::LastResult;
use minijinja::value::{Kwargs, Value as MiniValue};
use minijinja::{Environment, Error, ErrorKind};
use serde_json::{json, Value as JsonValue};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 8 * 1024;
const TEMPLATE_OUTPUT_TEMP_ATTEMPTS: usize = 16;

// These resource files are the Canon-owned interrogation prompt/instruction
// templates. User-authored expectation questions are runtime data inserted into
// the turn prompt template. The turn prompt is sent as turn input, not as the
// evaluator developerInstructions parameter.
// `resources/prompts/` also contains the base instructions included by
// `protocol::base`; the developer-instructions and turn prompt templates are
// included here because they are rendered with runtime check data.
const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_developer_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_turn_prompt.txt");

pub(crate) struct DeveloperInstructionsContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) template_output_dir: &'a Path,
    pub(crate) against_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
    pub(crate) expectation_instructions: &'a str,
    pub(crate) visible_scope: &'a [String],
    pub(crate) checked_file_count: usize,
    pub(crate) visible_file_count: usize,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn developer_instructions(
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    let num_invisible_files = context
        .checked_file_count
        .checked_sub(context.visible_file_count)
        .ok_or("visible file count exceeds checked file count")?;
    // The transcript intentionally has two levels of diff evidence: unscoped
    // `git diff --numstat` for change discovery, then scoped detailed `git
    // diff` for inspectable content. If the summary shows a changed path whose
    // details are outside `visible_scope`, the evaluator has enough context to
    // return ScopeTooNarrow rather than treating the path as nonexistent.
    render_minijinja_resource_template(
        context.root,
        context.template_output_dir,
        DEVELOPER_INSTRUCTIONS_TEMPLATE,
        json!({
            "expectation": {
                "instructions": context.expectation_instructions,
            },
            "against_tree_oid": context.against_tree_oid,
            "checked_tree_oid": context.checked_tree_oid,
            "last_pass": context.last_pass,
            "visible_scope": context.visible_scope,
            "num_invisible_files": num_invisible_files,
        }),
    )
}

pub(crate) fn evaluator_turn_prompt(
    root: &Path,
    template_output_dir: &Path,
    question: &str,
    expected_answer: &str,
    target: Option<&str>,
    last_pass: Option<&LastResult>,
) -> Result<String, String> {
    render_minijinja_resource_template(
        root,
        template_output_dir,
        EVALUATOR_TURN_PROMPT_TEMPLATE,
        json!({
            "question": question,
            "expectation": {
                "a": expected_answer,
                "target": target.unwrap_or(""),
            },
            "last_pass": last_pass,
        }),
    )
}

fn render_minijinja_resource_template(
    root: &Path,
    template_output_dir: &Path,
    template: &str,
    context: JsonValue,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    environment.add_filter("shargs", shell_args_filter);
    let command_root = root.to_path_buf();
    let command_output_dir = template_output_dir.to_path_buf();
    let sh_transcript_markers = ShTranscriptMarkers::new()?;
    let filter_transcript_markers = sh_transcript_markers.clone();
    environment.add_filter(
        "sh",
        move |command: String, kwargs: Kwargs| -> Result<String, Error> {
            let transcript =
                shell_transcript_filter(&command_root, &command_output_dir, command, kwargs)?;
            Ok(filter_transcript_markers.wrap_transcript(&transcript))
        },
    );
    let template = environment
        .template_from_str(template)
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    // Prompt Templates require the MiniJinja render itself to start from the
    // repository root cwd; use the shared process-cwd guard for that boundary.
    let rendered = render_with_repository_cwd(root, || template.render(context))
        .map_err(|err| format!("failed to render prompt template: {}", err))?;
    // Canon trims only outer template whitespace. Internal sentinels protect
    // `sh` transcript text at prompt boundaries so command display text,
    // stdout text, saved stdout bytes, and the truncation marker keep their
    // specified spelling.
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

pub(crate) fn create_prompt_template_output_dir() -> Result<PathBuf, String> {
    let temp_dir = std::env::temp_dir();
    for _ in 0..16 {
        let random = getrandom::u64()
            .map_err(|err| format!("failed to choose prompt template output dir: {err}"))?;
        let path = temp_dir.join(format!(
            "canon-template-output-{}-{random:016x}",
            std::process::id()
        ));
        match create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(_) if path.exists() => continue,
            Err(err) => return Err(format!("failed to create {}: {}", path.display(), err)),
        }
    }
    Err(format!(
        "failed to create a unique prompt template output dir under {}",
        temp_dir.display()
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
    command: String,
    kwargs: Kwargs,
) -> Result<String, Error> {
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
    let output = run_prompt_template_shell_command(root, &command)
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
    )?);
    if !transcript.ends_with('\n') {
        transcript.push('\n');
    }
    Ok(transcript)
}

fn truncated_template_command_output(
    output: &[u8],
    template_output_dir: &Path,
) -> Result<String, Error> {
    if output.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(String::from_utf8_lossy(output).into_owned());
    }
    // Prompt-template truncation is a token-budget mechanism, not a visibility
    // boundary. The Prompt Templates spec deliberately exposes a readable file
    // containing the same command stdout so the evaluator can inspect more of
    // an already-visible transcript when the head is insufficient.
    let path = write_full_template_output(template_output_dir, output)?;
    let (mut head, head_lines) = template_output_head(output);
    if !head.ends_with('\n') {
        head.push('\n');
    }
    let total_lines = output_line_count(output);
    head.push_str(&format!(
        "[truncated: showing first {} of {} lines; full output: {}]\n",
        head_lines,
        total_lines,
        path.display()
    ));
    Ok(head)
}

fn template_output_head(output: &[u8]) -> (String, usize) {
    let mut end = 0usize;
    let mut head_lines = 0usize;
    for line in output.split_inclusive(|byte| *byte == b'\n') {
        if end.saturating_add(line.len()) > TEMPLATE_OUTPUT_HEAD_BYTES {
            break;
        }
        end += line.len();
        head_lines += 1;
    }
    if head_lines > 0 {
        return (
            String::from_utf8_lossy(&output[..end]).into_owned(),
            head_lines,
        );
    }

    let end = TEMPLATE_OUTPUT_HEAD_BYTES.min(output.len());
    let head = String::from_utf8_lossy(&output[..end]).into_owned();
    let head_lines = output_line_count(&output[..end]);
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

fn write_full_template_output(template_output_dir: &Path, output: &[u8]) -> Result<PathBuf, Error> {
    fs::create_dir_all(template_output_dir).map_err(|err| {
        template_error(format!(
            "failed to create prompt template output dir {}: {}",
            template_output_dir.display(),
            err
        ))
    })?;
    for _ in 0..TEMPLATE_OUTPUT_TEMP_ATTEMPTS {
        let random = getrandom::u64().map_err(|err| {
            template_error(format!(
                "failed to choose prompt template output path: {}",
                err
            ))
        })?;
        let path = template_output_dir.join(format!("canon-template-output-{random:016x}.txt"));
        match create_template_output_file(&path, output) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(template_error(format!(
                    "failed to write {}: {}",
                    path.display(),
                    err
                )));
            }
        }
    }
    Err(template_error(format!(
        "failed to create a unique prompt template output file in {}",
        template_output_dir.display()
    )))
}

fn create_template_output_file(path: &Path, output: &[u8]) -> io::Result<()> {
    let mut file = template_output_open_options().open(path)?;
    let result = set_template_output_file_permissions(&file)
        .and_then(|()| file.write_all(output))
        .and_then(|()| file.flush());
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn template_output_open_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    set_template_output_create_mode(&mut options);
    options
}

#[cfg(unix)]
fn set_template_output_create_mode(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_template_output_create_mode(_options: &mut fs::OpenOptions) {}

#[cfg(unix)]
fn set_template_output_file_permissions(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_template_output_file_permissions(_file: &fs::File) -> io::Result<()> {
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
    fn template_command_output_truncates_large_output() {
        let output = (0..6000)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let output_dir = test_output_dir("truncate");
        let rendered = truncated_template_command_output(output.as_bytes(), &output_dir).unwrap();

        assert!(rendered.contains("[truncated: showing first "));
        assert!(rendered.contains("; full output: "));
        assert!(!rendered.contains("[begin untrusted command output"));
        assert!(!rendered.contains("[end untrusted command output"));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn template_command_output_file_preserves_raw_stdout_bytes() {
        let mut output = (0..1200)
            .flat_map(|index| format!("line {index}\n").into_bytes())
            .collect::<Vec<_>>();
        output.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let output_dir = test_output_dir("raw-bytes");

        let rendered = truncated_template_command_output(&output, &output_dir).unwrap();
        let truncation_line = rendered
            .lines()
            .find(|line| line.starts_with("[truncated: "))
            .unwrap();
        let path = truncation_line
            .strip_suffix(']')
            .unwrap()
            .rsplit_once("full output: ")
            .unwrap()
            .1;
        let saved = fs::read(path).unwrap();

        assert_eq!(saved, output);
        let saved_path = Path::new(path);
        assert_eq!(saved_path.parent(), Some(output_dir.as_path()));
        let _ = fs::remove_dir_all(output_dir);
    }

    fn test_output_dir(label: &str) -> PathBuf {
        let random = getrandom::u64().unwrap();
        std::env::temp_dir().join(format!(
            "canon-prompt-template-output-{label}-{}-{random:016x}",
            std::process::id()
        ))
    }

    #[test]
    fn target_diff_previous_response_uses_full_q_scope_suggestion() {
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

        let prompt = evaluator_turn_prompt(
            Path::new("."),
            &output_dir,
            "Does it pass?",
            "yes",
            Some("diff"),
            Some(&last_pass),
        )
        .unwrap();

        assert!(prompt.contains(r#""qScopeSuggestion": ["."]"#));
        assert!(!prompt.contains(r#""qScopeSuggestion": ["src/a.rs"]"#));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resource_template_rendering_trims_outer_whitespace() {
        let output_dir = test_output_dir("outer-trim");

        let rendered = render_minijinja_resource_template(
            Path::new("."),
            &output_dir,
            "\n  {{ value }}  \n",
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
