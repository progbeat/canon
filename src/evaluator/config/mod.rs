mod app_server;
mod codec;
mod permissions;
mod runtime;

pub(crate) use app_server::{app_server_args, AppServerArgs};
pub(crate) use runtime::{
    evaluator_reasoning_effort, evaluator_thread_config_identity,
    EphemeralEvaluatorThreadPermissionProfile, EvaluatorHostIsolation, EvaluatorProcessIsolation,
    EvaluatorRuntimeConfigContext, EvaluatorRuntimeConfigSnapshot, EvaluatorThreadConfigIdentity,
    EvaluatorThreadConfigIdentityContext,
};
use std::fmt;
use std::path::{Path, PathBuf};

const EVALUATOR_PERMISSION_PROFILE: &str = "canon_check";

pub(super) const EVALUATOR_DISABLED_FEATURES: &[&str] = &[
    "apps",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "fast_mode",
    "guardian_approval",
    "hooks",
    "image_generation",
    "in_app_browser",
    "multi_agent",
    "personality",
    "plugin_hooks",
    "shell_snapshot",
    "skill_mcp_dependency_install",
    "terminal_resize_reflow",
    "tool_call_mcp_elicitation",
    "tool_search",
    "tool_suggest",
    "unavailable_dummy_tools",
    "unified_exec",
    "workspace_dependencies",
];
pub(super) type EvaluatorConfigResult<T> = Result<T, EvaluatorConfigError>;

#[derive(Debug)]
pub(crate) enum EvaluatorConfigError {
    Message(String),
    DuplicateConfigEntry {
        path: String,
    },
    DuplicateFilesystemConfigEntry {
        path: String,
    },
    DuplicateFilesystemPermission {
        path: String,
        existing: String,
        replacement: String,
    },
    InvalidPathUtf8 {
        context: &'static str,
    },
    RuntimeInputInsideProtectedHostRoot {
        input: PathBuf,
        protected_root: PathBuf,
    },
    JsonEncode {
        context: &'static str,
        message: String,
    },
}

impl fmt::Display for EvaluatorConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvaluatorConfigError::Message(message) => formatter.write_str(message),
            EvaluatorConfigError::DuplicateConfigEntry { path } => {
                write!(formatter, "duplicate evaluator config entry for {}", path)
            }
            EvaluatorConfigError::DuplicateFilesystemConfigEntry { path } => {
                write!(
                    formatter,
                    "duplicate evaluator filesystem config entry for {}",
                    path
                )
            }
            EvaluatorConfigError::DuplicateFilesystemPermission {
                path,
                existing,
                replacement,
            } => write!(
                formatter,
                "duplicate evaluator filesystem permission for {}: {} and {}",
                path, existing, replacement
            ),
            EvaluatorConfigError::InvalidPathUtf8 { context } => {
                write!(formatter, "{} must be valid UTF-8", context)
            }
            EvaluatorConfigError::RuntimeInputInsideProtectedHostRoot {
                input,
                protected_root,
            } => write!(
                formatter,
                "evaluator runtime input {} must be outside protected host root {}",
                input.display(),
                protected_root.display()
            ),
            EvaluatorConfigError::JsonEncode { context, message } => {
                write!(formatter, "failed to encode {}: {}", context, message)
            }
        }
    }
}

impl std::error::Error for EvaluatorConfigError {}

impl From<String> for EvaluatorConfigError {
    fn from(message: String) -> EvaluatorConfigError {
        EvaluatorConfigError::Message(message)
    }
}

pub(super) fn path_to_config_string(
    path: &Path,
    context: &'static str,
) -> EvaluatorConfigResult<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or(EvaluatorConfigError::InvalidPathUtf8 { context })
}
