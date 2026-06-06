use minijinja::value::{Kwargs, Value};
use minijinja::{context, Environment, Error, ErrorKind};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 32 * 1024;

static TEMPLATE_OUTPUT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

// These resource files are the Canon-owned interrogation prompt/instruction
// templates. User-authored expectation questions are runtime data inserted into
// the turn prompt template.
const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../resources/prompts/evaluator_developer_instructions.txt");
const STATIC_DEVELOPER_INSTRUCTIONS: &str =
    include_str!("../../resources/prompts/evaluator_static_developer_instructions.txt");
pub(crate) const EVALUATOR_BASE_INSTRUCTIONS: &str =
    include_str!("../../resources/prompts/evaluator_base_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../resources/prompts/evaluator_turn_prompt.txt");

pub(crate) struct DeveloperInstructionsContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) against_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
    pub(crate) visible_scope: &'a [String],
    pub(crate) checked_file_count: usize,
    pub(crate) visible_file_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AgainstTreeAnswer {
    pub(crate) answer: String,
    pub(crate) evidence: String,
}

pub(crate) fn developer_instructions(
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    render_resource_template(
        context.root,
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        context! {
            static_developer_instructions => STATIC_DEVELOPER_INSTRUCTIONS.trim_end(),
            against_tree_oid => context.against_tree_oid,
            checked_tree_oid => context.checked_tree_oid,
            visible_scope => context.visible_scope,
            num_invisible_files => context.checked_file_count.saturating_sub(context.visible_file_count),
        },
    )
}

pub(crate) fn evaluator_turn_prompt(
    question: &str,
    against_tree_answer: Option<&AgainstTreeAnswer>,
) -> Result<String, String> {
    render_resource_template(
        Path::new("."),
        EVALUATOR_TURN_PROMPT_TEMPLATE.trim_end(),
        context! {
            question => question,
            against_tree_answer => against_tree_answer,
        },
    )
}

fn render_resource_template(
    root: &Path,
    template: &str,
    context: minijinja::Value,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    let command_root = root.to_path_buf();
    environment.add_filter(
        "sh",
        move |command: String, kwargs: Kwargs| -> Result<String, Error> {
            shell_transcript_filter(&command_root, command, kwargs)
        },
    );
    let template = environment
        .template_from_str(template)
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    template
        .render(context)
        .map_err(|err| format!("failed to render prompt template: {}", err))
}

fn json_filter(value: Value) -> Result<String, Error> {
    serde_json::to_string(&value).map_err(|err| template_error(err.to_string()))
}

fn shell_quote_filter(value: String) -> String {
    shell_quote(&value)
}

fn shell_transcript_filter(root: &Path, command: String, kwargs: Kwargs) -> Result<String, Error> {
    let display = kwargs
        .get::<Option<String>>("display")
        .map_err(|err| template_error(err.to_string()))?
        .unwrap_or_else(|| command.clone());
    kwargs
        .assert_all_used()
        .map_err(|err| template_error(err.to_string()))?;
    let output = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(root)
        .output()
        .map_err(|err| template_error(format!("failed to run prompt template command: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(template_error(format!(
            "prompt template command failed: {}",
            stderr.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut transcript = String::new();
    transcript.push_str("$ ");
    transcript.push_str(&display);
    transcript.push('\n');
    transcript.push_str(&truncated_template_command_output(&stdout)?);
    Ok(transcript)
}

fn truncated_template_command_output(output: &str) -> Result<String, Error> {
    if output.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(output.to_string());
    }
    let path = write_full_template_output(output)?;
    let mut head = String::new();
    let mut head_lines = 0usize;
    for line in output.split_inclusive('\n') {
        if head.len().saturating_add(line.len()) > TEMPLATE_OUTPUT_HEAD_BYTES {
            break;
        }
        head.push_str(line);
        head_lines += 1;
    }
    let total_lines = output.lines().count();
    head.push_str(&format!(
        "[truncated: showing first {} of {} lines; full output: {}]\n",
        head_lines,
        total_lines,
        path.display()
    ));
    Ok(head)
}

fn write_full_template_output(output: &str) -> Result<PathBuf, Error> {
    let sequence = TEMPLATE_OUTPUT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "canon-template-output-{}-{}.txt",
        std::process::id(),
        sequence
    ));
    fs::write(&path, output)
        .map_err(|err| template_error(format!("failed to write {}: {}", path.display(), err)))?;
    Ok(path)
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let mut quoted = String::new();
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn template_error(message: String) -> Error {
    Error::new(ErrorKind::InvalidOperation, message)
}

#[cfg(test)]
mod tests {
    use super::{
        developer_instructions, evaluator_turn_prompt, AgainstTreeAnswer,
        DeveloperInstructionsContext, EVALUATOR_BASE_INSTRUCTIONS,
    };
    use crate::git::{empty_tree_oid, staged_tree_oid};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn base_instructions_prohibit_status_text() {
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("Do not announce skills"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("only the JSON object"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("request a shell command or tool call"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains(r#"{"tool":...,"parameters":...}"#));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains(r#"{"command":...}"#));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains(r#"error:"insufficient-evidence""#));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("I'll inspect"));
    }

    #[test]
    fn developer_instructions_define_topic_neutral_evidence_threshold() {
        let root = git_project("developer-instructions-template");
        fs::write(root.join("file.txt"), "changed\n").unwrap();
        git(&root, &["add", "file.txt"]);
        let against_tree_oid = empty_tree_oid(&root).unwrap();
        let checked_tree_oid = staged_tree_oid(&root).unwrap();

        let instructions = developer_instructions(DeveloperInstructionsContext {
            root: &root,
            against_tree_oid: &against_tree_oid,
            checked_tree_oid: &checked_tree_oid,
            visible_scope: &[".".to_string()],
            checked_file_count: 1,
            visible_file_count: 1,
        })
        .unwrap();

        assert!(instructions.contains("visible files and question text do not prove"));
        assert!(instructions.contains("Relevant direct reads/searches"));
        assert!(instructions.contains("do not require a literal exhaustive audit"));
        assert!(!instructions.contains("answer `no` to"));
        assert!(instructions.contains("text before or after the JSON is invalid"));
        assert!(instructions.contains("tool-request JSON"));
        assert!(instructions.contains("Tool calls are not an output format"));
        assert!(instructions.contains(r#"{"tool":...}"#));
        assert!(instructions.contains(r#"{"command":...}"#));
        assert!(instructions.contains("first non-whitespace character must be `{`"));
        assert!(instructions.contains("leading inspection summaries"));
        assert!(instructions.contains("backslash immediately before a backtick"));
        assert!(instructions.contains("$ git diff --numstat --cached\n"));
        assert!(instructions.contains("$ sandbox --read-only --scope [\".\"]"));
        assert!(instructions.contains("Hidden files: 0."));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn turn_prompt_includes_against_tree_answer_when_available() {
        let prompt = evaluator_turn_prompt(
            "Does it pass?",
            Some(&AgainstTreeAnswer {
                answer: "yes".to_string(),
                evidence: "`src/main.rs` proves it".to_string(),
            }),
        )
        .unwrap();

        assert!(prompt.starts_with("Does it pass?"));
        assert!(prompt.contains("Your previous answer at HEAD:"));
        assert!(prompt.contains(r#""answer":"yes""#));
        assert!(prompt.contains(r#""evidence":"`src/main.rs` proves it""#));
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "core.eol", "lf"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
