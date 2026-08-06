use super::runtime::{
    shell_transcript_filter, PromptTemplateArtifactDir, ShTranscriptMarkers, ShellTranscriptContext,
};
use minijinja::value::{Kwargs, Object};
use minijinja::{Error, ErrorKind, State};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(super) const CONTEXT_NAME: &str = "__canon_prompt_shell_context";

pub(super) struct PromptShellContext {
    root: PathBuf,
    artifact_dir: PromptTemplateArtifactDir,
    artifact_paths: Arc<Mutex<Vec<PathBuf>>>,
    environment: Vec<(OsString, OsString)>,
    arguments: Vec<String>,
    transcript_markers: ShTranscriptMarkers,
}

impl PromptShellContext {
    pub(super) fn new(
        root: PathBuf,
        artifact_dir: PromptTemplateArtifactDir,
        artifact_paths: Arc<Mutex<Vec<PathBuf>>>,
        environment: Vec<(OsString, OsString)>,
        arguments: Vec<String>,
        transcript_markers: ShTranscriptMarkers,
    ) -> PromptShellContext {
        PromptShellContext {
            root,
            artifact_dir,
            artifact_paths,
            environment,
            arguments,
            transcript_markers,
        }
    }
}

impl fmt::Debug for PromptShellContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PromptShellContext")
    }
}

impl Object for PromptShellContext {}

pub(super) fn prompt_shell_filter(
    state: &State<'_, '_>,
    command: String,
    kwargs: Kwargs,
) -> Result<String, Error> {
    let context = state
        .lookup(CONTEXT_NAME)
        .and_then(|value| value.downcast_object::<PromptShellContext>())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidOperation,
                "prompt shell filter context is missing",
            )
        })?;
    let transcript = shell_transcript_filter(
        ShellTranscriptContext {
            root: &context.root,
            artifact_dir: &context.artifact_dir,
            artifact_paths: context.artifact_paths.as_ref(),
            environment: &context.environment,
            arguments: &context.arguments,
        },
        command,
        kwargs,
    )?;
    Ok(context.transcript_markers.wrap_transcript(&transcript))
}
