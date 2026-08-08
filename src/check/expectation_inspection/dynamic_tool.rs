//! The runtime `canon.show` adapter for expectation inspection.

use crate::check::ExpectationIdentity;
use crate::config_types::CheckConfig;
use crate::evaluator::{
    EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult,
};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::xpec_state::XpecStateCache;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use super::{render_show_for_current_run, ShowRenderRequest};

#[derive(Clone, Copy)]
pub(crate) struct CanonShowContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) identities: &'a [ExpectationIdentity],
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) current_expectation_id: Option<&'a str>,
}

pub(crate) struct CanonShowDynamicToolHandler<'context, 'state> {
    context: CanonShowContext<'context>,
    xpec_state: &'state mut XpecStateCache,
    visible_tree_oid_cache: &'state mut VisibleTreeOidCache,
    shown_expectation_ids: BTreeSet<String>,
}

impl<'context, 'state> CanonShowDynamicToolHandler<'context, 'state> {
    pub(crate) fn new(
        context: CanonShowContext<'context>,
        xpec_state: &'state mut XpecStateCache,
        visible_tree_oid_cache: &'state mut VisibleTreeOidCache,
    ) -> CanonShowDynamicToolHandler<'context, 'state> {
        CanonShowDynamicToolHandler {
            context,
            xpec_state,
            visible_tree_oid_cache,
            shown_expectation_ids: BTreeSet::new(),
        }
    }

    pub(crate) fn into_shown_expectation_ids(self) -> BTreeSet<String> {
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
            root: self.context.root,
            config: self.context.config,
            identities: self.context.identities,
            tree_source: self.context.tree_source,
            selectors: &selectors,
            pathspecs: &arguments.pathspecs,
            current_expectation_id: self.context.current_expectation_id,
            xpec_state: self.xpec_state,
            visible_tree_oid_cache: self.visible_tree_oid_cache,
        })?;
        self.shown_expectation_ids.extend(rendered.expectation_ids);
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
