mod artifact;
mod cwd;
mod filters;
mod transcript;

pub(super) use artifact::{PromptTemplateArtifactDir, PromptTemplateArtifactDirCache};
pub(super) use cwd::render_with_repository_cwd;
pub(super) use filters::{
    json_filter, shell_args_filter, shell_quote_filter, shell_transcript_filter,
    ShellTranscriptContext,
};
pub(super) use transcript::{trim_rendered_prompt_template_output, ShTranscriptMarkers};

use minijinja::{Error, ErrorKind};

fn template_error(message: String) -> Error {
    Error::new(ErrorKind::InvalidOperation, message)
}
