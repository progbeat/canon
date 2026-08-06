//! Capability-limited project inspection for evaluator agents.

mod cache;
mod filesystem;
mod operations;
mod output;

use self::cache::{EvaluatorProjectInspectionCache, ProjectInspectionCacheKey};
use super::tool_schema::load_dynamic_tools;
use super::{EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

const PROJECT_TOOL_NAMESPACE: &str = "project";
const PROJECT_TOOL_NAMES: [&str; 3] = ["files", "read", "search"];
const PROJECT_TOOL_RESOURCE: &str =
    include_str!("../../resources/prompts/evaluator_project_dynamic_tools.json");
static PROJECT_DYNAMIC_TOOLS: OnceLock<Result<Vec<Value>, String>> = OnceLock::new();

pub(super) fn evaluator_project_dynamic_tools() -> Result<Vec<Value>, String> {
    load_dynamic_tools(
        &PROJECT_DYNAMIC_TOOLS,
        PROJECT_TOOL_RESOURCE,
        "evaluator project",
    )
}

pub(super) fn evaluator_project_tools_are_advertised(dynamic_tools: &[Value]) -> bool {
    // [bP,KD,hQ] The shell is disabled only when the complete limited tool set
    // is present. Per-mode tests therefore observe any tool-list divergence.
    let names = evaluator_project_tool_names(dynamic_tools);
    names.len() == PROJECT_TOOL_NAMES.len()
        && names.into_iter().collect::<BTreeSet<_>>() == BTreeSet::from(PROJECT_TOOL_NAMES)
}

fn evaluator_project_tool_names(dynamic_tools: &[Value]) -> Vec<&str> {
    dynamic_tools
        .iter()
        .filter(|namespace| {
            namespace.get("name").and_then(Value::as_str) == Some(PROJECT_TOOL_NAMESPACE)
        })
        .filter_map(|namespace| namespace.get("tools").and_then(Value::as_array))
        .flatten()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect()
}

fn is_project_tool_name(name: &str) -> bool {
    PROJECT_TOOL_NAMES.contains(&name)
}

pub(super) struct EvaluatorProjectDynamicToolHandler<'a> {
    cwd: &'a Path,
    template_artifact_directory: &'a Path,
    cache: Option<EvaluatorProjectInspectionCache>,
}

impl<'a> EvaluatorProjectDynamicToolHandler<'a> {
    pub(super) fn for_immutable_snapshot(
        cwd: &'a Path,
        template_artifact_directory: &'a Path,
    ) -> EvaluatorProjectDynamicToolHandler<'a> {
        EvaluatorProjectDynamicToolHandler {
            cwd,
            template_artifact_directory,
            cache: Some(EvaluatorProjectInspectionCache::default()),
        }
    }

    pub(super) fn for_live_filesystem(
        cwd: &'a Path,
        template_artifact_directory: &'a Path,
    ) -> EvaluatorProjectDynamicToolHandler<'a> {
        EvaluatorProjectDynamicToolHandler {
            cwd,
            template_artifact_directory,
            cache: None,
        }
    }

    pub(super) fn handles(call: &EvaluatorDynamicToolCall) -> bool {
        call.namespace.as_deref() == Some(PROJECT_TOOL_NAMESPACE)
            && is_project_tool_name(&call.tool)
    }

    fn handle(&self, call: EvaluatorDynamicToolCall) -> Result<String, String> {
        let Some(cache) = &self.cache else {
            return self.handle_uncached(call);
        };
        let key = ProjectInspectionCacheKey {
            cwd: self.cwd.to_path_buf(),
            template_artifact_directory: self.template_artifact_directory.to_path_buf(),
            tool: call.tool.clone(),
            arguments: call.arguments.to_string(),
        };
        cache.result(key, || self.handle_uncached(call))
    }
}

impl EvaluatorDynamicToolHandler for EvaluatorProjectDynamicToolHandler<'_> {
    fn handle_dynamic_tool_call(
        &mut self,
        call: EvaluatorDynamicToolCall,
    ) -> EvaluatorDynamicToolResult {
        match self.handle(call) {
            Ok(output) => EvaluatorDynamicToolResult::success(output),
            Err(err) => EvaluatorDynamicToolResult::failure(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: bP,KD,hQ,l,qv
    fn evaluators_receive_only_read_only_project_tools() {
        let tools = evaluator_project_dynamic_tools().unwrap();
        let project = tools
            .iter()
            .find(|namespace| namespace["name"] == PROJECT_TOOL_NAMESPACE)
            .unwrap();
        let declared_tools = project["tools"].as_array().unwrap();
        let names = declared_tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let search = declared_tools
            .iter()
            .find(|tool| tool["name"] == "search")
            .unwrap();

        assert!(evaluator_project_tools_are_advertised(&tools));
        assert_eq!(tools.len(), 1);
        assert_eq!(names, BTreeSet::from(["files", "read", "search"]));
        assert_eq!(search["inputSchema"]["properties"]["query"]["minLength"], 1);
        assert_eq!(
            search["inputSchema"]["properties"]["query"]["maxLength"],
            1024
        );
    }
}
