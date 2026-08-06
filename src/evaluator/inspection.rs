//! Evaluator-owned inspection tool selection and routing.

use super::project_tool::{
    evaluator_project_dynamic_tools, evaluator_project_tools_are_advertised,
    EvaluatorProjectDynamicToolHandler,
};
use super::{
    EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult,
    EvaluatorProcessIsolation,
};
use serde_json::Value;
use std::path::Path;

#[derive(Clone)]
pub(crate) struct ReadOnlyProjectInspectionPlan {
    process_isolation: EvaluatorProcessIsolation,
    dynamic_tools: Vec<Value>,
}

impl ReadOnlyProjectInspectionPlan {
    pub(crate) fn for_process_isolation(
        process_isolation: EvaluatorProcessIsolation,
    ) -> Result<ReadOnlyProjectInspectionPlan, String> {
        let dynamic_tools = evaluator_project_dynamic_tools()?;
        let project_tools_advertised = evaluator_project_tools_are_advertised(&dynamic_tools);
        if !project_tools_advertised {
            return Err("evaluator read-only project tool set is incomplete".to_string());
        }
        Ok(ReadOnlyProjectInspectionPlan {
            process_isolation,
            dynamic_tools,
        })
    }

    pub(crate) fn dynamic_tools(&self) -> &[Value] {
        &self.dynamic_tools
    }

    pub(crate) fn process_isolation(&self) -> EvaluatorProcessIsolation {
        self.process_isolation
    }

    pub(crate) fn advertises_project_tools(dynamic_tools: &[Value]) -> bool {
        evaluator_project_tools_are_advertised(dynamic_tools)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum EvaluatorProjectFilesystem {
    ImmutableSnapshot,
    LiveReadOnlyInspection,
}

pub(crate) struct EvaluatorInspectionDynamicToolHandler<'fallback, 'inputs> {
    project_tools_advertised: bool,
    project: EvaluatorProjectDynamicToolHandler<'inputs>,
    fallback: Option<&'fallback mut dyn EvaluatorDynamicToolHandler>,
}

impl<'fallback, 'inputs> EvaluatorInspectionDynamicToolHandler<'fallback, 'inputs> {
    pub(crate) fn new(
        project_tools_advertised: bool,
        cwd: &'inputs Path,
        template_artifact_directory: &'inputs Path,
        project_filesystem: EvaluatorProjectFilesystem,
        fallback: Option<&'fallback mut dyn EvaluatorDynamicToolHandler>,
    ) -> EvaluatorInspectionDynamicToolHandler<'fallback, 'inputs> {
        let project = match project_filesystem {
            // [d] Snapshot contents are stable inputs, so identical expensive
            // inspections may be reused within the evaluator turn.
            EvaluatorProjectFilesystem::ImmutableSnapshot => {
                EvaluatorProjectDynamicToolHandler::for_immutable_snapshot(
                    cwd,
                    template_artifact_directory,
                )
            }
            // [90,KD,l] Live inspection refreshes read-only tool results from
            // the project. "Live" describes freshness, not write access.
            EvaluatorProjectFilesystem::LiveReadOnlyInspection => {
                EvaluatorProjectDynamicToolHandler::for_live_filesystem(
                    cwd,
                    template_artifact_directory,
                )
            }
        };
        EvaluatorInspectionDynamicToolHandler {
            project_tools_advertised,
            project,
            fallback,
        }
    }
}

impl EvaluatorDynamicToolHandler for EvaluatorInspectionDynamicToolHandler<'_, '_> {
    fn handle_dynamic_tool_call(
        &mut self,
        call: EvaluatorDynamicToolCall,
    ) -> EvaluatorDynamicToolResult {
        if self.project_tools_advertised && EvaluatorProjectDynamicToolHandler::handles(&call) {
            return self.project.handle_dynamic_tool_call(call);
        }
        match self.fallback.as_deref_mut() {
            Some(fallback) => fallback.handle_dynamic_tool_call(call),
            None => EvaluatorDynamicToolResult::failure(
                "dynamic tool calls are not available for this evaluator turn",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::filesystem::{
        OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
    };
    use serde_json::json;
    use std::fs;

    #[test] // xpec: bP,KD,hQ,l
    fn every_process_isolation_mode_advertises_read_only_project_tools() {
        let managed = ReadOnlyProjectInspectionPlan::for_process_isolation(
            EvaluatorProcessIsolation::CanonManaged,
        )
        .unwrap();
        let external = ReadOnlyProjectInspectionPlan::for_process_isolation(
            EvaluatorProcessIsolation::ExternallyManaged,
        )
        .unwrap();

        assert!(ReadOnlyProjectInspectionPlan::advertises_project_tools(
            managed.dynamic_tools()
        ));
        assert!(ReadOnlyProjectInspectionPlan::advertises_project_tools(
            external.dynamic_tools()
        ));
    }

    #[test] // xpec: 90,KD,d
    fn live_filesystem_inspection_reads_current_content_on_each_call() {
        let temporary = OwnedPrivateTemporaryDirectory::create(
            &PrivateTemporaryDirectoryAllocator::new(),
            "canon-live-inspection-test",
        )
        .unwrap();
        let project = temporary.path().join("project");
        let artifacts = temporary.path().join("artifacts");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(project.join("status.txt"), "old\n").unwrap();
        let inspection_plan = ReadOnlyProjectInspectionPlan::for_process_isolation(
            EvaluatorProcessIsolation::ExternallyManaged,
        )
        .unwrap();
        let mut handler = EvaluatorInspectionDynamicToolHandler::new(
            ReadOnlyProjectInspectionPlan::advertises_project_tools(
                inspection_plan.dynamic_tools(),
            ),
            &project,
            &artifacts,
            EvaluatorProjectFilesystem::LiveReadOnlyInspection,
            None,
        );
        let read_call = || EvaluatorDynamicToolCall {
            namespace: Some("project".to_string()),
            tool: "read".to_string(),
            arguments: json!({ "path": "status.txt" }),
        };

        let first = handler.handle_dynamic_tool_call(read_call());
        fs::write(project.join("status.txt"), "new\n").unwrap();
        let second = handler.handle_dynamic_tool_call(read_call());

        assert!(first.text.contains("old"));
        assert!(second.text.contains("new"));
        assert!(!second.text.contains("old"));
    }
}
