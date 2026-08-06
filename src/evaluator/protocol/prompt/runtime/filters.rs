use super::artifact::{truncated_template_command_output, PromptTemplateArtifactDir};
use super::template_error;
use crate::evaluator::protocol::prompt_shell::{
    quote_prompt_template_shell_arg, run_prompt_template_shell_command,
};
use minijinja::value::{Kwargs, Value as MiniValue};
use minijinja::Error;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) fn json_filter(value: MiniValue) -> Result<String, Error> {
    serde_json::to_string(&value).map_err(|err| template_error(err.to_string()))
}

pub(crate) fn shell_quote_filter(value: String) -> Result<String, Error> {
    quote_prompt_template_shell_arg(&value).map_err(template_error)
}

pub(crate) fn shell_args_filter(value: MiniValue) -> Result<String, Error> {
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

pub(crate) struct ShellTranscriptContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) artifact_dir: &'a PromptTemplateArtifactDir,
    pub(crate) artifact_paths: &'a Mutex<Vec<PathBuf>>,
    pub(crate) environment: &'a [(OsString, OsString)],
    pub(crate) arguments: &'a [String],
}

pub(crate) fn shell_transcript_filter(
    context: ShellTranscriptContext<'_>,
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
    // supplied check root without mutating the parent process cwd.
    let output = run_prompt_template_shell_command(
        context.root,
        &command,
        context.environment,
        context.arguments,
    )
    .map_err(|err| template_error(format!("failed to run prompt template command: {err}")))?;
    if !output.status.success() {
        return Err(template_error(format!(
            "prompt template command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = output.stdout;
    let mut transcript = String::new();
    transcript.push_str("$ ");
    transcript.push_str(&display);
    transcript.push('\n');
    // The transcript shape is exactly command line, command stdout, and
    // optionally the single truncation marker appended below; no extra
    // begin/end sentinel lines are part of the Prompt Templates contract.
    transcript.push_str(&truncated_template_command_output(
        &stdout,
        context.artifact_dir,
        context.artifact_paths,
    )?);
    if !transcript.ends_with('\n') {
        transcript.push('\n');
    }
    Ok(transcript)
}
