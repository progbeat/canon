use super::transport::{apply_local_turn_environment, AppServerTurnRequest};
use super::AppServerRunner;
use crate::evaluator::{
    evaluator_reasoning_effort, EvaluatorDynamicToolHandler, EvaluatorError,
    EvaluatorInspectionDynamicToolHandler, EvaluatorProcessIsolation,
};
use crate::token_usage::EvaluatorTurnUsage;
use serde_json::{json, Value};
use std::path::Path;

mod process_isolation;
mod telemetry;
mod thread_start;

use process_isolation::turn_sandbox_policy;
pub(super) use thread_start::{AppServerThreadStartContext, InvocationThreadStartMemo};

impl AppServerRunner {
    pub(super) fn ask(
        &mut self,
        thread_id: &str,
        rendered_turn_text: &str,
        model: Option<&str>,
        thinking: &str,
        output_schema: &Value,
        dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
    ) -> Result<String, EvaluatorError> {
        let request = turn_start_request(
            thread_id,
            rendered_turn_text,
            model,
            thinking,
            output_schema,
            self.thread_cwd(thread_id),
            self.process_isolation(),
        )?;
        let request = AppServerTurnRequest::new(thread_id, request);
        let inputs = self
            .thread_runtime_inputs(thread_id)
            .cloned()
            .ok_or_else(|| EvaluatorError::message("missing evaluator thread runtime inputs"))?;
        let mut handler = EvaluatorInspectionDynamicToolHandler::new(
            inputs.project_tools_advertised,
            &inputs.cwd,
            &inputs.template_artifact_directory,
            self.project_filesystem,
            dynamic_tool_handler,
        );
        self.send_turn_request("turn/start", request, Some(&mut handler))
    }

    pub(super) fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.take_last_turn_usage_record()
    }
}

fn turn_start_request(
    thread_id: &str,
    rendered_turn_text: &str,
    model: Option<&str>,
    thinking: &str,
    output_schema: &Value,
    cwd: Option<&Path>,
    process_isolation: EvaluatorProcessIsolation,
) -> Result<Value, EvaluatorError> {
    // Enforce the Interrogation Policy response schema selected by this
    // interrogation's q-scope. For q-scope ["."], the schema excludes
    // ScopeTooNarrow before the evaluator turn is started. This is the
    // one-turn request boundary; shared check/ask retry and q-scope
    // verification orchestration lives in
    // `src/check/engine/execute/expectation/policy.rs`. The q-scope
    // verification gate lives in `src/check/interrogation/policy/q_scope.rs`.
    let mut request = json!({
        "threadId": thread_id,
        "input": [
            {
                "type": "text",
                "text": rendered_turn_text
            }
        ],
        "outputSchema": output_schema.clone()
    });
    if let Some(cwd) = cwd {
        apply_local_turn_environment(&mut request, cwd)?;
    }
    apply_turn_process_isolation(process_isolation, &mut request);
    if let Some(model) = model {
        request["model"] = Value::String(model.to_string());
    }
    if let Some(effort) = evaluator_reasoning_effort(thinking) {
        request["effort"] = Value::String(effort.to_string());
    }
    Ok(request)
}

fn apply_turn_process_isolation(process_isolation: EvaluatorProcessIsolation, request: &mut Value) {
    // Canon-managed turns keep the named profile selected by thread/start.
    // Replacing it with the legacy read-only policy would discard the named
    // profile's narrow read and deny rules. External isolation deliberately
    // selects danger-full-access instead.
    if let Some(policy) = turn_sandbox_policy(process_isolation) {
        request["sandboxPolicy"] = policy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: bP,hQ
    fn canon_managed_turn_inherits_the_named_permission_contract() {
        let request = turn_start_request(
            "thread",
            "turn text",
            None,
            "medium",
            &json!({}),
            Some(Path::new("/cwd")),
            EvaluatorProcessIsolation::CanonManaged,
        )
        .unwrap();

        // [hQ] Omitting turn-level overrides is the public protocol behavior
        // that preserves the named thread permission profile.
        assert!(request.get("permissions").is_none());
        assert!(request.get("sandboxPolicy").is_none());
    }

    #[test] // xpec: bP,hQ
    fn externally_isolated_turn_selects_danger_full_access() {
        let request = turn_start_request(
            "thread",
            "turn text",
            None,
            "medium",
            &json!({}),
            None,
            EvaluatorProcessIsolation::ExternallyManaged,
        )
        .unwrap();

        assert_eq!(
            request.get("sandboxPolicy"),
            Some(&json!({ "type": "dangerFullAccess" }))
        );
        assert!(request.get("permissions").is_none());
    }
}
