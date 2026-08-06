use super::filesystem::{
    existing_entry_without_symlink_traversal, read_bounded, require_file_entry,
    validated_relative_path, validated_relative_path_value, walk_file_entries,
};
use super::output::{finish_bounded_output, project_label, push_bounded_line};
use super::EvaluatorProjectDynamicToolHandler;
use crate::evaluator::EvaluatorDynamicToolCall;
use serde::Deserialize;
use serde_json::Value;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

const READ_LIMIT_BYTES: u64 = 256 * 1024;
const SEARCH_FILE_LIMIT_BYTES: u64 = 1024 * 1024;
const READ_LINE_LIMIT: u64 = 400;
const SEARCH_QUERY_LIMIT_CHARS: usize = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesArguments {
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArguments {
    path: String,
    start_line: Option<NonZeroU64>,
    end_line: Option<NonZeroU64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArguments {
    query: String,
    path: Option<String>,
}

impl EvaluatorProjectDynamicToolHandler<'_> {
    pub(super) fn handle_uncached(&self, call: EvaluatorDynamicToolCall) -> Result<String, String> {
        match call.tool.as_str() {
            "files" => {
                let arguments = parse_arguments(call.arguments, "project.files")?;
                self.list_files(arguments)
            }
            "read" => {
                let arguments = parse_arguments(call.arguments, "project.read")?;
                self.read_file(arguments)
            }
            "search" => {
                let arguments = parse_arguments(call.arguments, "project.search")?;
                self.search_files(arguments)
            }
            _ => Err("unknown project inspection tool".to_string()),
        }
    }

    fn list_files(&self, arguments: FilesArguments) -> Result<String, String> {
        let relative = validated_relative_path(arguments.path.as_deref().unwrap_or("."))?;
        let mut output = String::new();
        let truncated = walk_file_entries(self.cwd, &relative, |path| {
            Ok(push_bounded_line(
                &mut output,
                &project_label(self.cwd, path),
            ))
        })?;
        finish_bounded_output(output, truncated)
    }

    fn read_file(&self, arguments: ReadArguments) -> Result<String, String> {
        let (root, relative, label) = self.resolve_read_path(&arguments.path)?;
        let path = existing_entry_without_symlink_traversal(root, &relative)?;
        require_file_entry(&path)?;
        let (bytes, content_truncated) = read_bounded(&path, READ_LIMIT_BYTES)?;
        let text = String::from_utf8_lossy(&bytes);
        let start_line = arguments.start_line.map(NonZeroU64::get).unwrap_or(1);
        let requested_end = arguments
            .end_line
            .map(NonZeroU64::get)
            .unwrap_or_else(|| start_line.saturating_add(READ_LINE_LIMIT).saturating_sub(1));
        if requested_end < start_line {
            return Err("project.read endLine must not precede startLine".to_string());
        }
        let end_line =
            requested_end.min(start_line.saturating_add(READ_LINE_LIMIT).saturating_sub(1));
        let mut output = String::new();
        let mut output_truncated = requested_end > end_line;
        for (index, line) in text.lines().enumerate() {
            let line_number = index as u64 + 1;
            if line_number < start_line {
                continue;
            }
            if line_number > end_line {
                output_truncated = true;
                break;
            }
            if !push_bounded_line(&mut output, &format!("{label}:{line_number}: {line}")) {
                output_truncated = true;
                break;
            }
        }
        if output.is_empty() {
            push_bounded_line(
                &mut output,
                &format!("{label}: no lines in requested range"),
            );
        }
        finish_bounded_output(output, output_truncated || content_truncated)
    }

    fn search_files(&self, arguments: SearchArguments) -> Result<String, String> {
        if arguments.query.is_empty() {
            return Err("project.search query must not be empty".to_string());
        }
        if arguments.query.chars().count() > SEARCH_QUERY_LIMIT_CHARS {
            return Err("project.search query exceeds 1024 Unicode characters".to_string());
        }
        let relative = validated_relative_path(arguments.path.as_deref().unwrap_or("."))?;
        let mut output = String::new();
        let mut output_truncated = false;
        let traversal_truncated = walk_file_entries(self.cwd, &relative, |path| {
            let (bytes, file_truncated) = read_bounded(path, SEARCH_FILE_LIMIT_BYTES)?;
            let label = project_label(self.cwd, path);
            for (index, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if line.contains(&arguments.query)
                    && !push_bounded_line(&mut output, &format!("{label}:{}: {line}", index + 1))
                {
                    output_truncated = true;
                    return Ok(false);
                }
            }
            if file_truncated {
                output_truncated = true;
            }
            Ok(true)
        })?;
        if output.is_empty() {
            output.push_str("no matches\n");
        }
        finish_bounded_output(output, output_truncated || traversal_truncated)
    }

    fn resolve_read_path(&self, value: &str) -> Result<(&Path, PathBuf, String), String> {
        let path = Path::new(value);
        if path.is_absolute() {
            let relative = path
                .strip_prefix(self.template_artifact_directory)
                .map_err(|_| {
                    "project.read absolute paths must name a Canon prompt artifact".to_string()
                })?;
            let relative = validated_relative_path_value(relative)?;
            return Ok((
                self.template_artifact_directory,
                relative,
                value.to_string(),
            ));
        }
        let relative = validated_relative_path(value)?;
        let label = relative.to_string_lossy().replace('\\', "/");
        Ok((self.cwd, relative, label))
    }
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(
    arguments: Value,
    tool: &str,
) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|err| format!("invalid {tool} arguments: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::project_tool::PROJECT_TOOL_NAMESPACE;
    use crate::evaluator::EvaluatorDynamicToolHandler;
    use crate::platform::filesystem::{
        OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
    };
    use serde_json::json;
    use std::fs;

    fn temporary_roots() -> (OwnedPrivateTemporaryDirectory, PathBuf, PathBuf) {
        let temporary = OwnedPrivateTemporaryDirectory::create(
            &PrivateTemporaryDirectoryAllocator::new(),
            "canon-project-tool-test",
        )
        .unwrap();
        let project = temporary.path().join("project");
        let artifacts = temporary.path().join("artifacts");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(project.join("nested/.git")).unwrap();
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(project.join("src/lib.rs"), "first\nneedle\n").unwrap();
        fs::write(project.join(".git/secret"), "hidden\n").unwrap();
        fs::write(project.join("nested/.git/secret"), "needle-hidden\n").unwrap();
        fs::write(artifacts.join("full-output"), "artifact\n").unwrap();
        (temporary, project, artifacts)
    }

    #[test] // xpec: bP,KD,hQ
    fn project_tools_read_and_search_only_declared_roots() {
        let (_temporary, project, artifacts) = temporary_roots();
        let mut handler =
            EvaluatorProjectDynamicToolHandler::for_live_filesystem(&project, &artifacts);
        let search = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "search".to_string(),
            arguments: json!({ "query": "needle" }),
        });
        let artifact = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "read".to_string(),
            arguments: json!({ "path": artifacts.join("full-output") }),
        });
        let escape = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "read".to_string(),
            arguments: json!({ "path": "/etc/passwd" }),
        });
        let git_admin_aliases = [".git", ".GIT", ".git.", ".git "].map(|alias| {
            handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
                namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
                tool: "read".to_string(),
                arguments: json!({ "path": format!("nested/{alias}/secret") }),
            })
        });

        assert!(search.success);
        assert!(search.text.contains("src/lib.rs:2: needle"));
        assert!(!search.text.contains(".git"));
        assert!(!search.text.contains("needle-hidden"));
        assert!(artifact.success);
        assert!(artifact.text.contains("artifact"));
        assert!(!escape.success);
        assert!(escape.text.contains("must name a Canon prompt artifact"));
        for result in git_admin_aliases {
            assert!(!result.success);
            assert!(result
                .text
                .contains("does not expose Git administrative files"));
        }
    }

    #[test] // xpec: qv,hQ
    fn project_read_rejects_line_zero_like_its_schema() {
        let (_temporary, project, artifacts) = temporary_roots();
        let mut handler =
            EvaluatorProjectDynamicToolHandler::for_live_filesystem(&project, &artifacts);

        for arguments in [
            json!({ "path": "src/lib.rs", "startLine": 0 }),
            json!({ "path": "src/lib.rs", "endLine": 0 }),
        ] {
            let result = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
                namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
                tool: "read".to_string(),
                arguments,
            });

            assert!(!result.success);
            assert!(result.text.contains("invalid project.read arguments"));
        }
    }

    #[test] // xpec: qv,hQ
    fn project_search_enforces_its_schema_length_in_unicode_characters() {
        let (_temporary, project, artifacts) = temporary_roots();
        let mut handler =
            EvaluatorProjectDynamicToolHandler::for_live_filesystem(&project, &artifacts);

        let accepted = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "search".to_string(),
            arguments: json!({ "query": "é".repeat(SEARCH_QUERY_LIMIT_CHARS) }),
        });
        let rejected = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "search".to_string(),
            arguments: json!({ "query": "é".repeat(SEARCH_QUERY_LIMIT_CHARS + 1) }),
        });

        assert!(accepted.success);
        assert!(!rejected.success);
        assert!(rejected.text.contains("exceeds 1024 Unicode characters"));
    }

    #[cfg(unix)]
    #[test] // xpec: 90,KD
    fn project_tools_expose_symlink_entries_without_following_them() {
        use std::os::unix::fs::symlink;

        let (_temporary, project, artifacts) = temporary_roots();
        let outside = project.parent().unwrap().join("outside-secret");
        fs::write(&outside, "outside-secret-content").unwrap();
        symlink(&outside, project.join("src/link")).unwrap();
        let mut handler =
            EvaluatorProjectDynamicToolHandler::for_live_filesystem(&project, &artifacts);

        let files = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "files".to_string(),
            arguments: json!({}),
        });
        let read = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "read".to_string(),
            arguments: json!({ "path": "src/link" }),
        });
        let search = handler.handle_dynamic_tool_call(EvaluatorDynamicToolCall {
            namespace: Some(PROJECT_TOOL_NAMESPACE.to_string()),
            tool: "search".to_string(),
            arguments: json!({ "query": "outside-secret-content" }),
        });

        assert!(files.success);
        assert!(files.text.contains("src/link"));
        assert!(read.success);
        assert!(read.text.contains("outside-secret"));
        assert!(!read.text.contains("outside-secret-content"));
        assert!(search.success);
        assert_eq!(search.text, "no matches\n");
    }
}
