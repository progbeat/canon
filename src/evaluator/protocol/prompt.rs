use minijinja::value::{Kwargs, Value};
use minijinja::{context, Environment, Error, ErrorKind};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const TEMPLATE_OUTPUT_HEAD_BYTES: usize = 8 * 1024;

static TEMPLATE_OUTPUT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

// These resource files are the Canon-owned interrogation prompt/instruction
// templates. User-authored expectation questions are runtime data inserted into
// the turn prompt template.
const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_developer_instructions.txt");
// Runtime value for the developer template's `static_developer_instructions`
// variable; it is rendered by this module but is not a separate template.
const STATIC_DEVELOPER_INSTRUCTIONS: &str =
    include_str!("../../../resources/prompts/evaluator_static_developer_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_turn_prompt.txt");

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
    let num_invisible_files = context
        .checked_file_count
        .checked_sub(context.visible_file_count)
        .ok_or("visible file count exceeds checked file count")?;
    // The canon template displays this transcript as `git diff --numstat --cached`;
    // back that with a temporary index containing the checked tree.
    let prompt_diff_index = PromptDiffIndex::from_tree(context.root, context.checked_tree_oid)?;
    render_minijinja_resource_template_with_git_index(
        context.root,
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        Some(prompt_diff_index.path()),
        context! {
            static_developer_instructions => STATIC_DEVELOPER_INSTRUCTIONS.trim_end(),
            against_tree_oid => "--cached",
            checked_tree_oid => context.against_tree_oid,
            visible_scope => context.visible_scope,
            num_invisible_files => num_invisible_files,
        },
    )
}

pub(crate) fn evaluator_turn_prompt(
    root: &Path,
    question: &str,
    against_tree_answer: Option<&AgainstTreeAnswer>,
) -> Result<String, String> {
    render_minijinja_resource_template(
        root,
        EVALUATOR_TURN_PROMPT_TEMPLATE.trim_end(),
        context! {
            question => question,
            against_tree_answer => against_tree_answer,
        },
    )
}

fn render_minijinja_resource_template(
    root: &Path,
    template: &str,
    context: minijinja::Value,
) -> Result<String, String> {
    render_minijinja_resource_template_with_git_index(root, template, None, context)
}

fn render_minijinja_resource_template_with_git_index(
    root: &Path,
    template: &str,
    git_index_file: Option<&Path>,
    context: minijinja::Value,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    let command_root = root.to_path_buf();
    let command_git_index_file = git_index_file.map(Path::to_path_buf);
    environment.add_filter(
        "sh",
        move |command: String, kwargs: Kwargs| -> Result<String, Error> {
            shell_transcript_filter(
                &command_root,
                command_git_index_file.as_deref(),
                command,
                kwargs,
            )
        },
    );
    let template = environment
        .template_from_str(template)
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    let rendered = render_template_from_root(root, || template.render(context))?;
    rendered.map_err(|err| format!("failed to render prompt template: {}", err))
}

fn render_template_from_root<T>(root: &Path, render: impl FnOnce() -> T) -> Result<T, String> {
    let previous =
        std::env::current_dir().map_err(|err| format!("failed to read current dir: {err}"))?;
    std::env::set_current_dir(root).map_err(|err| {
        format!(
            "failed to enter prompt template root {}: {}",
            root.display(),
            err
        )
    })?;
    let rendered = render();
    std::env::set_current_dir(&previous).map_err(|err| {
        format!(
            "failed to restore current dir {}: {}",
            previous.display(),
            err
        )
    })?;
    Ok(rendered)
}

fn json_filter(value: Value) -> Result<String, Error> {
    serde_json::to_string(&value).map_err(|err| template_error(err.to_string()))
}

fn shell_quote_filter(value: String) -> String {
    shell_quote(&value)
}

fn shell_transcript_filter(
    root: &Path,
    git_index_file: Option<&Path>,
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
    let mut process = Command::new("sh");
    process.arg("-c").arg(&command).current_dir(root);
    if let Some(git_index_file) = git_index_file {
        process.env("GIT_INDEX_FILE", git_index_file);
    }
    let output = process
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

struct PromptDiffIndex {
    path: PathBuf,
}

impl PromptDiffIndex {
    fn from_tree(root: &Path, checked_tree_oid: &str) -> Result<PromptDiffIndex, String> {
        let sequence = TEMPLATE_OUTPUT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "canon-prompt-index-{}-{}",
            std::process::id(),
            sequence
        ));
        let output = Command::new("git")
            .arg("read-tree")
            .arg(checked_tree_oid)
            .env("GIT_INDEX_FILE", &path)
            .current_dir(root)
            .output()
            .map_err(|err| format!("failed to prepare prompt diff index: {err}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "failed to prepare prompt diff index: {}",
                stderr.trim()
            ));
        }
        Ok(PromptDiffIndex { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PromptDiffIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn truncated_template_command_output(output: &str) -> Result<String, Error> {
    if output.len() <= TEMPLATE_OUTPUT_HEAD_BYTES {
        return Ok(output.to_string());
    }
    let path = write_full_template_output(output)?;
    let (mut head, head_lines) = template_output_head(output);
    if !head.ends_with('\n') {
        head.push('\n');
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

fn template_output_head(output: &str) -> (String, usize) {
    let mut head = String::new();
    let mut head_lines = 0usize;
    for line in output.split_inclusive('\n') {
        if head.len().saturating_add(line.len()) > TEMPLATE_OUTPUT_HEAD_BYTES {
            break;
        }
        head.push_str(line);
        head_lines += 1;
    }
    if head_lines > 0 {
        return (head, head_lines);
    }

    let mut end = TEMPLATE_OUTPUT_HEAD_BYTES.min(output.len());
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    let head = output[..end].to_string();
    let head_lines = head.lines().count();
    (head, head_lines)
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
        developer_instructions, evaluator_turn_prompt, truncated_template_command_output,
        AgainstTreeAnswer, DeveloperInstructionsContext, STATIC_DEVELOPER_INSTRUCTIONS,
        TEMPLATE_OUTPUT_HEAD_BYTES,
    };
    use crate::git::{empty_tree_oid, staged_tree_oid};
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn developer_instructions_render_required_diff_and_visible_scope_context() {
        let _lock = prompt_render_lock();
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

        assert!(instructions.contains("$ git diff --numstat --cached\n"));
        assert!(instructions
            .trim_start()
            .starts_with(STATIC_DEVELOPER_INSTRUCTIONS.trim_end()));
        assert!(instructions.contains("$ sandbox --read-only --scope [\".\"]"));
        assert!(instructions.contains("Hidden files: 0."));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn developer_instructions_prompt_diff_uses_checked_tree_not_real_index() {
        let _lock = prompt_render_lock();
        let root = git_project("developer-instructions-prompt-diff-index");
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        let against_tree_oid = staged_tree_oid(&root).unwrap();
        fs::write(root.join("tracked.txt"), "checked\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        let checked_tree_oid = staged_tree_oid(&root).unwrap();
        fs::write(root.join("other.txt"), "real index only\n").unwrap();
        git(&root, &["add", "other.txt"]);

        let instructions = developer_instructions(DeveloperInstructionsContext {
            root: &root,
            against_tree_oid: &against_tree_oid,
            checked_tree_oid: &checked_tree_oid,
            visible_scope: &[".".to_string()],
            checked_file_count: 1,
            visible_file_count: 1,
        })
        .unwrap();

        assert!(instructions.contains("$ git diff --numstat --cached\n"));
        assert!(instructions.contains("1\t1\ttracked.txt\n"));
        assert!(!instructions.contains("other.txt"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn turn_prompt_renders_against_tree_answer_protocol_section() {
        let _lock = prompt_render_lock();
        let prompt = evaluator_turn_prompt(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "Does it pass?",
            Some(&AgainstTreeAnswer {
                answer: "yes".to_string(),
                evidence: "`src/main.rs` proves it".to_string(),
            }),
        )
        .unwrap();

        let answer_json = prompt
            .split_once("Your previous answer at HEAD:")
            .map(|(_, answer)| answer.trim())
            .expect("prompt should include the against-tree answer section");
        let answer: Value = serde_json::from_str(answer_json).unwrap();

        assert_eq!(prompt.lines().next(), Some("Does it pass?"));
        assert_eq!(answer["answer"], "yes");
        assert_eq!(answer["evidence"], "`src/main.rs` proves it");
    }

    #[test]
    fn long_template_output_renders_required_truncation_notice_after_head() {
        let output = "x".repeat(TEMPLATE_OUTPUT_HEAD_BYTES + 1);

        let rendered = truncated_template_command_output(&output).unwrap();

        assert!(rendered.starts_with('x'));
        assert_truncation_notice_points_to_full_output(&rendered, &output);
    }

    #[test]
    fn long_template_output_renders_required_complete_head_line_count() {
        let output = format!("first line\n{}", "x".repeat(TEMPLATE_OUTPUT_HEAD_BYTES + 1));

        let rendered = truncated_template_command_output(&output).unwrap();

        assert!(rendered.starts_with("first line\n"));
        assert!(!rendered.contains(&"x".repeat(TEMPLATE_OUTPUT_HEAD_BYTES)));
        assert_truncation_notice_points_to_full_output(&rendered, &output);
    }

    fn assert_truncation_notice_points_to_full_output(rendered: &str, output: &str) {
        let notice = rendered
            .lines()
            .find(|line| line.starts_with("[truncated: "))
            .expect("long template output should include a truncation notice");
        let path = notice
            .strip_suffix(']')
            .and_then(|line| line.rsplit_once("full output: "))
            .map(|(_, path)| Path::new(path))
            .expect("truncation notice should include a full-output path");
        assert_eq!(fs::read_to_string(path).unwrap(), output);
        let _ = fs::remove_file(path);
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

    fn prompt_render_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
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
