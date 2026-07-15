use crate::check::interrogation::state::CheckRuntime;
use crate::check::show::{render_show_for_current_run, ShowRenderRequest};
use crate::evaluator::{
    EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult,
};
use crate::git::VisibleTreeOidCache;
use crate::xpec_state::XpecStateCache;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::OsString;

pub(super) fn canon_show_dynamic_tools() -> Vec<Value> {
    vec![json!({
        "type": "namespace",
        "name": "canon",
        "description": "Inspect expectations from the current canon check run.",
        "tools": [
            {
                "type": "function",
                "name": "show",
                "description": "Return canon show output for requested expectation ID prefixes or full IDs in the current check run.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "selectors": {
                            "description": "Expectation selectors: ID prefix/full ID or not:<ID prefix/full ID>.",
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "pathspecs": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "additionalProperties": false
                }
            }
        ]
    })]
}

pub(super) struct CanonShowDynamicToolHandler<'runtime, 'state> {
    runtime: &'runtime CheckRuntime<'runtime>,
    current_expectation_id: Option<&'runtime str>,
    xpec_state: &'state mut XpecStateCache,
    visible_tree_oid_cache: &'state mut VisibleTreeOidCache,
    shown_expectation_ids: BTreeSet<String>,
}

impl<'runtime, 'state> CanonShowDynamicToolHandler<'runtime, 'state> {
    pub(super) fn new(
        runtime: &'runtime CheckRuntime<'runtime>,
        current_expectation_id: Option<&'runtime str>,
        xpec_state: &'state mut XpecStateCache,
        visible_tree_oid_cache: &'state mut VisibleTreeOidCache,
    ) -> CanonShowDynamicToolHandler<'runtime, 'state> {
        CanonShowDynamicToolHandler {
            runtime,
            current_expectation_id,
            xpec_state,
            visible_tree_oid_cache,
            shown_expectation_ids: BTreeSet::new(),
        }
    }

    pub(super) fn into_shown_expectation_ids(self) -> BTreeSet<String> {
        self.shown_expectation_ids
    }
}

impl EvaluatorDynamicToolHandler for CanonShowDynamicToolHandler<'_, '_> {
    fn handle_dynamic_tool_call(
        &mut self,
        call: EvaluatorDynamicToolCall,
    ) -> EvaluatorDynamicToolResult {
        if call.namespace.as_deref() != Some("canon") || call.tool != "show" {
            return EvaluatorDynamicToolResult::failure("unknown dynamic tool");
        }
        match self.render_show(call.arguments) {
            Ok(text) => EvaluatorDynamicToolResult::success(text),
            Err(err) => EvaluatorDynamicToolResult::failure(err),
        }
    }
}

impl CanonShowDynamicToolHandler<'_, '_> {
    fn render_show(&mut self, arguments: Value) -> Result<String, String> {
        let arguments = parse_canon_show_arguments(arguments)?;
        let selectors = arguments
            .selectors
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        let rendered = render_show_for_current_run(ShowRenderRequest {
            root: self.runtime.root,
            config: self.runtime.config,
            tree_source: self.runtime.tree_source(),
            selectors: &selectors,
            pathspecs: &arguments.pathspecs,
            // xpec: F
            // `canon.show` acts as if `not:<current expectation>` was added
            // after the evaluator-supplied selectors.
            excluded_expectation_id: self.current_expectation_id,
            xpec_state: self.xpec_state,
            visible_tree_oid_cache: self.visible_tree_oid_cache,
        })?;
        self.shown_expectation_ids
            .extend(rendered.expectation_ids.iter().cloned());
        Ok(rendered.text)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonShowArguments {
    #[serde(default)]
    selectors: Vec<String>,
    #[serde(default)]
    pathspecs: Vec<String>,
}

fn parse_canon_show_arguments(value: Value) -> Result<CanonShowArguments, String> {
    let arguments = serde_json::from_value::<CanonShowArguments>(value)
        .map_err(|err| format!("invalid canon.show arguments: {}", err))?;
    if arguments
        .pathspecs
        .iter()
        .any(|pathspec| pathspec.is_empty())
    {
        return Err("pathspec must not be empty".to_string());
    }
    Ok(arguments)
}
