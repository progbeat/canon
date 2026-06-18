mod app_server;
mod base;
mod codec;
mod model_catalog;
mod permissions;

pub(crate) use app_server::{app_server_args_with_no_sandbox, app_server_model_key, AppServerArgs};
pub(crate) use base::evaluator_thread_config_with_no_sandbox;
use std::fmt;
use std::path::Path;

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
pub(super) const EVALUATOR_EXTRA_DISABLED_FEATURES: &[&str] = &["apply_patch_freeform"];

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
    HomeNotUtf8,
    InvalidPathUtf8 {
        context: &'static str,
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
            EvaluatorConfigError::HomeNotUtf8 => {
                formatter.write_str("HOME must be valid UTF-8 for evaluator runtime permissions")
            }
            EvaluatorConfigError::InvalidPathUtf8 { context } => {
                write!(formatter, "{} must be valid UTF-8", context)
            }
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
